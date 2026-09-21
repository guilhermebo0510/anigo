/**
 * ANIGO — exportação canônica (P0 "Consolidar o renderer" / §8).
 *
 * Este serviço é a única porta de exportação do editor. Ele não gera geometria:
 * pede o artefato ao núcleo (`core_export_glb` / `core_export_frame`), valida o
 * manifesto contra o contrato (`export_manifest.v1.ts`) e **compara** o
 * manifesto com a geometria que o viewport está desenhando.
 *
 * É exatamente o critério de aceite "viewport e exportação passarem teste de
 * paridade": a comparação é feita em três camadas independentes —
 *
 * 1. identidade da geometria base (contagens, stride, `topology_hash`,
 *    `catalog_fingerprint`, checksum dos bytes de 72 B e dos índices);
 * 2. pesos de morph (o que o núcleo aplicou no arquivo tem de ser o que está
 *    nos canais do viewport);
 * 3. skinning (número de ossos e checksum da paleta) e, quando o backend é o
 *    caminho CPU (WebGL2), a malha deformada acumulada pelo viewport.
 *
 * Nada aqui recalcula a malha: `applyDeltasCpu` já é o caminho canônico de
 * acumulação de deltas do WebGL2 (os deltas e pesos vêm do snapshot do núcleo),
 * e os checksums são funções de bytes.
 */

import {
  EXPORT_FORMAT_VERSION,
  EXPORT_VERTEX_STRIDE_BYTES,
  ExportManifestError,
  indexChecksum,
  packedPositionChecksum,
  packedVertexChecksum,
  paletteChecksum,
  readExportManifest,
  type ExportManifestWire,
  type ExportRenderInfoWire,
} from "../contracts/export_manifest.v1";
import { CoreBridgeError, type CoreInvoker } from "./core_bridge";
import { applyDeltasCpu, type ViewportGeometry } from "./viewport_mesh";

/** Códigos estáveis de problema de paridade (a UI agrupa por eles). */
export type ExportParityCode =
  | "geometry_missing"
  | "vertex_count"
  | "index_count"
  | "triangle_count"
  | "vertex_stride"
  | "topology_hash"
  | "catalog_fingerprint"
  | "mesh_uri"
  | "base_vertex_checksum"
  | "base_index_checksum"
  | "morph_weights"
  | "static_revision"
  | "authority"
  | "bone_count"
  | "palette_checksum"
  | "deformed_vertex_checksum"
  | "deformed_position_checksum";

export interface ExportParityProblem {
  code: ExportParityCode;
  field: string;
  message: string;
  expected?: string;
  actual?: string;
}

export interface ExportParityReport {
  ok: boolean;
  problems: ExportParityProblem[];
  /** Campos conferidos com sucesso (útil no relatório de exportação). */
  checks: string[];
  /**
   * Campos que **não** puderam ser conferidos neste backend, com o motivo
   * (ex.: malha deformada no WebGPU, onde quem soma é o compute shader).
   */
  skipped: string[];
}

export interface ExportParityInput {
  manifest: unknown;
  /** Geometria que o viewport está desenhando (`null` = modo degradado). */
  geometry: ViewportGeometry | null;
  /** Revisão estática que o viewport recebeu (comparada com a do manifesto). */
  staticRevision?: number;
  /**
   * Confere também a malha deformada acumulando os deltas do núcleo no CPU
   * (o caminho que o WebGL2 usa). Desligado por padrão: no WebGPU quem soma é o
   * compute shader, e a comparação de bytes só é possível no caminho CPU.
   */
  verifyDeformed?: boolean;
  /** Autoridade esperada da deformação (default: `core`). */
  authority?: string;
}

/**
 * Compara o manifesto exportado com o estado do viewport.
 *
 * Não lança: problemas são dados (`problems`), para que a UI possa exportar,
 * mostrar o relatório e agir. Manifesto fora do contrato, sim, lança
 * `ExportManifestError` (é falha de formato, não divergência de conteúdo).
 */
export function checkExportParity(input: ExportParityInput): ExportParityReport {
  const manifest = readExportManifest(input.manifest);
  const problems: ExportParityProblem[] = [];
  const checks: string[] = [];
  const skipped: string[] = [];

  const expect = (
    code: ExportParityCode,
    field: string,
    expected: string | number,
    actual: string | number,
    message: string
  ): void => {
    if (String(expected) === String(actual)) {
      checks.push(field);
      return;
    }
    problems.push({
      code,
      field,
      message,
      expected: String(expected),
      actual: String(actual),
    });
  };

  const expectedAuthority = input.authority ?? "core";
  expect(
    "authority",
    "deformation.authority",
    expectedAuthority,
    manifest.deformation.authority,
    `a exportação precisa vir do núcleo (authority '${expectedAuthority}')`
  );

  const geometry = input.geometry;
  if (!geometry) {
    problems.push({
      code: "geometry_missing",
      field: "geometry",
      message:
        "viewport sem geometria canônica (modo degradado): não há o que comparar com o arquivo exportado",
    });
    return { ok: false, problems, checks, skipped };
  }

  expect(
    "vertex_count",
    "geometry.vertex_count",
    manifest.geometry.vertex_count,
    geometry.vertexCount,
    "contagem de vértices do arquivo diverge da geometria desenhada"
  );
  expect(
    "index_count",
    "geometry.index_count",
    manifest.geometry.index_count,
    geometry.indexCount,
    "contagem de índices do arquivo diverge da geometria desenhada"
  );
  expect(
    "triangle_count",
    "geometry.triangle_count",
    manifest.geometry.triangle_count,
    geometry.triangleCount,
    "contagem de triângulos do arquivo diverge da geometria desenhada"
  );
  expect(
    "vertex_stride",
    "geometry.vertex_stride_bytes",
    manifest.geometry.vertex_stride_bytes,
    EXPORT_VERTEX_STRIDE_BYTES,
    "stride do vértice do arquivo diverge do layout canônico"
  );
  expect(
    "topology_hash",
    "geometry.topology_hash",
    manifest.geometry.topology_hash,
    geometry.topologyHash,
    "hash de topologia do arquivo diverge da geometria desenhada"
  );
  expect(
    "catalog_fingerprint",
    "geometry.catalog_fingerprint",
    manifest.geometry.catalog_fingerprint,
    geometry.catalogFingerprint,
    "catálogo de morphs do arquivo diverge do que o viewport usa"
  );
  expect(
    "mesh_uri",
    "geometry.mesh_uri",
    manifest.geometry.mesh_uri,
    geometry.meshUri,
    "URI da malha do arquivo diverge da geometria desenhada"
  );

  // Bytes canônicos: é a comparação que pega qualquer divergência numérica.
  expect(
    "base_vertex_checksum",
    "geometry.base_vertex_checksum",
    manifest.geometry.base_vertex_checksum,
    packedVertexChecksum(geometry.vertices, geometry.vertexCount),
    "bytes dos vértices da base divergem entre o arquivo exportado e o viewport"
  );
  expect(
    "base_index_checksum",
    "geometry.base_index_checksum",
    manifest.geometry.base_index_checksum,
    indexChecksum(geometry.indices),
    "bytes dos índices divergem entre o arquivo exportado e o viewport"
  );

  if (input.staticRevision !== undefined) {
    expect(
      "static_revision",
      "project.static_revision",
      manifest.project.static_revision,
      input.staticRevision,
      "o arquivo foi exportado de uma revisão estática diferente da que o viewport desenha"
    );
  }

  // Pesos de morph: o que o núcleo aplicou no arquivo tem de ser o que está nos
  // canais do viewport (mesma regra de esparsidade: |w| > 1e-6).
  const geometryWeights = new Map<string, number>(
    geometry.channels.map((channel) => [channel.sliderId, channel.weight])
  );
  const exportedWeights = new Map<string, number>(
    manifest.deformation.morph_weights.map((entry) => [entry.slider_id, entry.weight])
  );
  const weightMismatch = compareWeights(geometryWeights, exportedWeights);
  if (weightMismatch) {
    problems.push(weightMismatch);
  } else {
    checks.push("deformation.morph_weights");
  }

  // Skinning: a paleta exportada tem de ser a mesma que o viewport subiu.
  expect(
    "bone_count",
    "skin.bone_count",
    manifest.skin.bone_count,
    geometry.boneCount,
    "o arquivo foi exportado com outro número de ossos"
  );
  expect(
    "palette_checksum",
    "skin.palette_checksum",
    manifest.skin.palette_checksum,
    paletteChecksum(geometry.skinPalette),
    "a paleta de ossos do arquivo diverge da que o viewport usa"
  );

  if (input.verifyDeformed) {
    const deformed = applyDeltasCpu(geometry, exportedWeights);
    expect(
      "deformed_vertex_checksum",
      "deformation.deformed_vertex_checksum",
      manifest.deformation.deformed_vertex_checksum,
      packedVertexChecksum(deformed, geometry.vertexCount),
      "a malha deformada acumulada pelo viewport difere da exportada pelo núcleo"
    );
    expect(
      "deformed_position_checksum",
      "deformation.deformed_position_checksum",
      manifest.deformation.deformed_position_checksum,
      packedPositionChecksum(deformed, geometry.vertexCount),
      "as posições deformadas acumuladas pelo viewport diferem das exportadas"
    );
  } else {
    skipped.push(
      "deformation.deformed_vertex_checksum e deformed_position_checksum (backend GPU: a soma é feita no compute shader)"
    );
  }

  return { ok: problems.length === 0, problems, checks, skipped };
}

function compareWeights(
  geometryWeights: ReadonlyMap<string, number>,
  exportedWeights: ReadonlyMap<string, number>
): ExportParityProblem | null {
  const sliders = new Set([...geometryWeights.keys(), ...exportedWeights.keys()]);
  for (const slider of sliders) {
    const drawn = geometryWeights.get(slider) ?? 0;
    const exported = exportedWeights.get(slider) ?? 0;
    if (Math.abs(drawn - exported) > 1e-6) {
      return {
        code: "morph_weights",
        field: `deformation.morph_weights.${slider}`,
        message: "peso de morph aplicado no arquivo diverge do que o viewport desenha",
        expected: String(exported),
        actual: String(drawn),
      };
    }
  }
  return null;
}

// ---------------------------------------------------------------------------
// Chamadas ao núcleo
// ---------------------------------------------------------------------------

/** Resposta de `core_export_glb` (o arquivo é escrito pelo núcleo). */
export interface CoreExportGlbResponseWire {
  glb_path: string;
  manifest_path: string;
  glb_bytes: number;
  glb_checksum: string;
  manifest: unknown;
}

/** Resposta de `core_export_frame` (PNG + métricas do renderer canônico). */
export interface CoreExportFrameResponseWire {
  image_path: string;
  manifest_path: string;
  image_bytes: number;
  adapter_name: string;
  backend: string;
  draw_calls: number;
  triangle_count: number;
  render_time_ms: number;
  manifest: unknown;
}

export interface ExportArtifactResult {
  glbPath: string;
  manifestPath: string;
  glbBytes: number;
  glbChecksum: string;
  manifest: ExportManifestWire;
  parity: ExportParityReport;
}

export interface FrameArtifactResult {
  imagePath: string;
  manifestPath: string;
  imageBytes: number;
  render: ExportRenderInfoWire;
  manifest: ExportManifestWire;
  parity: ExportParityReport;
}

export class ExportServiceError extends Error {
  readonly code: "core_unavailable" | "invoke_failed" | "invalid_response";
  readonly detail: string;
  constructor(code: ExportServiceError["code"], detail: string) {
    super(`[ANIGO export ${code}] ${detail}`);
    this.name = "ExportServiceError";
    this.code = code;
    this.detail = detail;
  }
}

/** Nomes de comando no núcleo (contrato com `src-tauri/src/main.rs`). */
export const EXPORT_COMMANDS = {
  glb: "core_export_glb",
  frame: "core_export_frame",
} as const;

export interface ExportGlbOptions {
  /** Invoker do núcleo (`null` = sem núcleo: browser/dev server). */
  invoker: CoreInvoker | null;
  /** Caminho completo do `.glb`; o manifesto vai ao lado (`.manifest.json`). */
  path: string;
  geometry: ViewportGeometry | null;
  staticRevision?: number;
  verifyDeformed?: boolean;
}

/**
 * Exporta o GLB canônico e confere a paridade com o viewport.
 *
 * O arquivo é escrito pelo **núcleo** (mesma malha deformada do snapshot); o
 * TypeScript só escolhe o caminho, valida o manifesto e compara.
 */
export async function exportCanonicalGlb(options: ExportGlbOptions): Promise<ExportArtifactResult> {
  if (!options.invoker) {
    throw new ExportServiceError(
      "core_unavailable",
      "sem núcleo canônico (o export exige a sessão Rust: Tauri/desktop)"
    );
  }
  const raw = await invokeCore(options.invoker, EXPORT_COMMANDS.glb, { path: options.path });
  const response = asRecord(raw, "core_export_glb");
  const manifest = readExportManifest(response.manifest);
  const parity = checkExportParity({
    manifest,
    geometry: options.geometry,
    staticRevision: options.staticRevision,
    verifyDeformed: options.verifyDeformed,
  });
  return {
    glbPath: requireStringField(response, "glb_path"),
    manifestPath: requireStringField(response, "manifest_path"),
    glbBytes: requireNumberField(response, "glb_bytes"),
    glbChecksum: requireStringField(response, "glb_checksum"),
    manifest,
    parity,
  };
}

export interface ExportFrameOptions {
  invoker: CoreInvoker | null;
  /** Caminho completo do `.png`; o manifesto vai ao lado. */
  path: string;
  width: number;
  height: number;
  geometry: ViewportGeometry | null;
  staticRevision?: number;
}

/**
 * Renderiza o frame de exportação com o renderer canônico (`anigo-renderer`,
 * os mesmos shaders/contrato do viewport) e grava PNG + manifesto.
 */
export async function exportCanonicalFrame(
  options: ExportFrameOptions
): Promise<FrameArtifactResult> {
  if (!options.invoker) {
    throw new ExportServiceError(
      "core_unavailable",
      "sem núcleo canônico (o frame de exportação exige a sessão Rust: Tauri/desktop)"
    );
  }
  const raw = await invokeCore(options.invoker, EXPORT_COMMANDS.frame, {
    path: options.path,
    width: Math.round(options.width),
    height: Math.round(options.height),
  });
  const response = asRecord(raw, "core_export_frame");
  const manifest = readExportManifest(response.manifest);
  const render = manifest.render;
  if (!render) {
    throw new ExportServiceError(
      "invalid_response",
      "o núcleo devolveu um frame sem a seção de render no manifesto"
    );
  }
  const parity = checkExportParity({
    manifest,
    geometry: options.geometry,
    staticRevision: options.staticRevision,
  });
  return {
    imagePath: requireStringField(response, "image_path"),
    manifestPath: requireStringField(response, "manifest_path"),
    imageBytes: requireNumberField(response, "image_bytes"),
    render,
    manifest,
    parity,
  };
}

/** Nome de arquivo sugerido para o artefato (determinístico, sem timestamp). */
export function exportFileName(
  projectName: string,
  revisions: { staticRevision: number; dynamicRevision: number },
  extension: "glb" | "png"
): string {
  const slug = projectName
    .replace(/\.anigo$/i, "")
    .normalize("NFD")
    .replace(/[\u0300-\u036f]/g, "")
    .replace(/[^a-zA-Z0-9._-]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .toLowerCase();
  const base = slug.length > 0 ? slug : "anigo";
  return `${base}-s${Math.max(0, Math.round(revisions.staticRevision))}-d${Math.max(
    0,
    Math.round(revisions.dynamicRevision)
  )}.${extension}`;
}

/** Relatório legível para a UI/telemetria. */
export function describeParity(report: ExportParityReport): string {
  if (report.ok) {
    const skipped = report.skipped.length > 0 ? `, ${report.skipped.length} não conferível(is) neste backend` : "";
    return `paridade ok (${report.checks.length} campos conferidos, contrato v${EXPORT_FORMAT_VERSION}${skipped})`;
  }
  const detail = report.problems
    .map((problem) => `${problem.field}: ${problem.message} (arquivo ${problem.expected} × viewport ${problem.actual})`)
    .join("; ");
  return report.skipped.length > 0 ? `${detail} — não conferido: ${report.skipped.join("; ")}` : detail;
}

async function invokeCore(
  invoker: CoreInvoker,
  command: string,
  args: Record<string, unknown>
): Promise<unknown> {
  try {
    return await invoker(command, args);
  } catch (error) {
    if (error instanceof CoreBridgeError) throw error;
    throw new ExportServiceError(
      "invoke_failed",
      `${command} falhou: ${error instanceof Error ? error.message : String(error)}`
    );
  }
}

function asRecord(value: unknown, command: string): Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new ExportServiceError("invalid_response", `${command} não devolveu um objeto`);
  }
  return value as Record<string, unknown>;
}

function requireStringField(source: Record<string, unknown>, key: string): string {
  const value = source[key];
  if (typeof value !== "string" || value.length === 0) {
    throw new ExportServiceError("invalid_response", `campo '${key}' ausente na resposta do núcleo`);
  }
  return value;
}

function requireNumberField(source: Record<string, unknown>, key: string): number {
  const value = source[key];
  if (typeof value !== "number" || !Number.isFinite(value)) {
    throw new ExportServiceError("invalid_response", `campo '${key}' inválido na resposta do núcleo`);
  }
  return value;
}

export { ExportManifestError };
