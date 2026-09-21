/**
 * ANIGO — validação de malha renderizável (P1 — Robustez, item 3).
 *
 * Regra: **nenhum** buffer de GPU é criado a partir de uma malha que não passou
 * por aqui (GLB, malha base ou geometria canônica do núcleo). Assim uma malha
 * corrompida vira diagnóstico recuperável (`mesh_invalid`/`geometry_rejected`)
 * em vez de um `ValidationError` do WebGPU no meio de um frame — ou, pior, de um
 * buffer com lixo que só aparece como artefato visual.
 *
 * Os códigos vêm do render contract (`mesh_validation.codes`), o mesmo documento
 * que o Rust lê em `crates/anigo-renderer/src/mesh_validation.rs`.
 */
import { RENDER_CONTRACT } from "../contracts/render_contract.v1";

/** Códigos de reprovação de malha declarados no contrato. */
export const MESH_VALIDATION_CODES = [
  "EMPTY_MESH",
  "INDEX_COUNT_MISMATCH",
  "INDEX_OUT_OF_RANGE",
  "NON_FINITE_VALUE",
  "BAD_VERTEX_STRIDE",
] as const;

export type MeshValidationCode = (typeof MESH_VALIDATION_CODES)[number];

/** Stride do vértice NPR canônico (72 B) declarado no contrato. */
export const VERTEX_STRIDE_BYTES: number =
  RENDER_CONTRACT.mesh_validation?.stride_bytes ?? 72;

/** Erro de malha reprovada (estruturado, com código estável). */
export class MeshValidationError extends Error {
  readonly code: MeshValidationCode;
  readonly detail: string;
  constructor(code: MeshValidationCode, detail: string) {
    super(`[MESH ${code}] ${detail}`);
    this.name = "MeshValidationError";
    this.code = code;
    this.detail = detail;
  }
}

export interface MeshValidationResult {
  ok: boolean;
  code: MeshValidationCode | null;
  message: string;
  vertexCount: number;
  indexCount: number;
}

/** Diferenças entre a lista local e a declarada no render contract. */
export function meshValidationCatalogProblems(): string[] {
  const declared = (RENDER_CONTRACT.mesh_validation?.codes ?? []).map((entry) => entry.code);
  const problems: string[] = [];
  for (const code of MESH_VALIDATION_CODES) {
    if (!declared.includes(code)) {
      problems.push(`código '${code}' usado no validador mas ausente do render contract`);
    }
  }
  for (const code of declared) {
    if (!(MESH_VALIDATION_CODES as readonly string[]).includes(code)) {
      problems.push(`código '${code}' declarado no contrato mas desconhecido do validador`);
    }
  }
  return problems;
}

/**
 * Valida a malha **antes** de criar buffers.
 *
 * `vertices` é o buffer cru empacotado no stride canônico (72 B: posição,
 * normal, uv, cor, joints, pesos). Quando a malha já está em arrays tipados
 * por atributo (caminho do GLB), use `validateAttributeMesh`.
 */
export function validateRenderableMesh(
  packed: ArrayBufferView,
  indices: ArrayBufferView,
  options: { strideBytes?: number } = {}
): MeshValidationResult {
  const stride = options.strideBytes ?? VERTEX_STRIDE_BYTES;
  const bytes = packed.byteLength;
  const indexCount = Math.floor(indices.byteLength / 4);
  const vertexCount = bytes / stride;

  if (bytes === 0 || indices.byteLength === 0) {
    return fail("EMPTY_MESH", `malha vazia (${bytes} B de vértice, ${indices.byteLength} B de índice)`);
  }
  if (stride <= 0 || bytes % stride !== 0) {
    return fail(
      "BAD_VERTEX_STRIDE",
      `${bytes} B de vértice não é múltiplo do stride de ${stride} B`
    );
  }
  if (indices.byteLength % 4 !== 0) {
    return fail("INDEX_COUNT_MISMATCH", `${indices.byteLength} B de índice não forma u32`);
  }
  if (indexCount % 3 !== 0) {
    return fail("INDEX_COUNT_MISMATCH", `${indexCount} índices não formam triângulos`);
  }

  const indexArray =
    indices instanceof Uint32Array
      ? indices
      : new Uint32Array(indices.buffer, indices.byteOffset, indexCount);
  for (let i = 0; i < indexArray.length; i += 1) {
    if (indexArray[i] >= vertexCount) {
      return fail(
        "INDEX_OUT_OF_RANGE",
        `índice[${i}] = ${indexArray[i]} aponta para fora de ${vertexCount} vértices`
      );
    }
  }

  const floats =
    packed instanceof Float32Array
      ? packed
      : new Float32Array(packed.buffer, packed.byteOffset, Math.floor(bytes / 4));
  for (let i = 0; i < floats.length; i += 1) {
    if (!Number.isFinite(floats[i])) {
      return fail("NON_FINITE_VALUE", `float[${i}] = ${floats[i]} (NaN/Infinito)`);
    }
  }

  return {
    ok: true,
    code: null,
    message: `malha ok: ${vertexCount} vértices, ${indexCount} índices`,
    vertexCount,
    indexCount,
  };
}

/**
 * Valida a malha decodificada por atributo (caminho do GLB).
 *
 * As posições/normais precisam ser finitas: um NaN em 4070 vértices destrói o
 * depth buffer e aparece como triângulo esticado na tela, não como erro.
 */
export function validateAttributeMesh(
  positions: ArrayLike<number>,
  indices: ArrayLike<number>,
  extra: { normals?: ArrayLike<number>; uvs?: ArrayLike<number> } = {}
): MeshValidationResult {
  const vertexCount = Math.floor(positions.length / 3);
  const indexCount = indices.length;

  if (vertexCount === 0 || indexCount === 0) {
    return fail(
      "EMPTY_MESH",
      `malha vazia (${vertexCount} vértices, ${indexCount} índices)`
    );
  }
  if (positions.length % 3 !== 0) {
    return fail("BAD_VERTEX_STRIDE", `${positions.length} floats de posição não formam vec3`);
  }
  if (extra.normals && extra.normals.length !== positions.length) {
    return fail(
      "BAD_VERTEX_STRIDE",
      `${extra.normals.length} floats de normal para ${positions.length} de posição`
    );
  }
  if (extra.uvs && extra.uvs.length !== vertexCount * 2) {
    return fail(
      "BAD_VERTEX_STRIDE",
      `${extra.uvs.length} floats de uv para ${vertexCount} vértices`
    );
  }
  if (indexCount % 3 !== 0) {
    return fail("INDEX_COUNT_MISMATCH", `${indexCount} índices não formam triângulos`);
  }
  for (let i = 0; i < indexCount; i += 1) {
    if (indices[i] >= vertexCount) {
      return fail(
        "INDEX_OUT_OF_RANGE",
        `índice[${i}] = ${indices[i]} aponta para fora de ${vertexCount} vértices`
      );
    }
  }
  for (let i = 0; i < positions.length; i += 1) {
    if (!Number.isFinite(positions[i])) {
      return fail("NON_FINITE_VALUE", `posição[${i}] = ${positions[i]} (NaN/Infinito)`);
    }
  }
  if (extra.normals) {
    for (let i = 0; i < extra.normals.length; i += 1) {
      if (!Number.isFinite(extra.normals[i])) {
        return fail("NON_FINITE_VALUE", `normal[${i}] = ${extra.normals[i]} (NaN/Infinito)`);
      }
    }
  }

  return {
    ok: true,
    code: null,
    message: `malha ok: ${vertexCount} vértices, ${indexCount} índices`,
    vertexCount,
    indexCount,
  };
}

/** Igual a `validateRenderableMesh`, mas lança `MeshValidationError`. */
export function assertRenderableMesh(
  packed: ArrayBufferView,
  indices: ArrayBufferView,
  options: { strideBytes?: number } = {}
): void {
  const result = validateRenderableMesh(packed, indices, options);
  if (!result.ok && result.code) throw new MeshValidationError(result.code, result.message);
}

function fail(code: MeshValidationCode, message: string): MeshValidationResult {
  return { ok: false, code, message, vertexCount: 0, indexCount: 0 };
}
