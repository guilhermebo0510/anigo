/**
 * ANIGO — geometria do viewport a partir do snapshot canônico
 * (ARQUITETURA_CANONICA_ANIGO §7, P0 itens 4–5).
 *
 * O viewport **não deforma** a malha canônica: ele recebe o snapshot do núcleo
 * (malha base preparada + canais de morph esparsos + pesos) e transforma isso em
 * buffers prontos para upload. A soma dos deltas acontece no shader de compute
 * canônico (`morph_sparse_compute.wgsl`), com os deltas que o núcleo gerou — não
 * existe geometria autoral do lado TypeScript.
 *
 * Todo layout aqui é validado contra o contrato congelado
 * (`contracts/fixtures/core_snapshot_v1.json`) antes de chegar na GPU: strides,
 * índices, limites de canal e a ordenação por `vertex_index` de que a busca
 * binária do shader depende.
 */

import {
  MORPH_CHANNEL_STRIDE_BYTES,
  MORPH_DELTA_STRIDE_BYTES,
  SnapshotContractError,
  VERTEX_STRIDE_BYTES,
  decodeCoreSnapshot,
  type CoreGeometry,
  type CoreSnapshotWire,
} from "../contracts/core_snapshot.v1";

/** Um canal de morph pronto para o buffer de canais do shader. */
export interface ViewportMorphChannel {
  /** Morph target do núcleo (`target` do payload). */
  target: string;
  /** Slider canônico (`slider_id` do payload). */
  sliderId: string;
  /** Peso do canal, vindo da parte dinâmica do snapshot. */
  weight: number;
  startOffset: number;
  deltaCount: number;
}

/** Geometria pronta para upload (nenhum byte recalculado). */
export interface ViewportGeometry {
  /** Vértices no layout canônico de 72 B (18 floats), idênticos ao núcleo. */
  vertices: Float32Array;
  /** Índices de triângulo. */
  indices: Uint32Array;
  vertexCount: number;
  indexCount: number;
  triangleCount: number;
  /** Canais em ordem de catálogo (só os com peso != 0 vão para a GPU ativa). */
  channels: ViewportMorphChannel[];
  /** Registros de 16 B por canal: `{weight, start_offset, delta_count, pad}`. */
  channelRecords: Float32Array;
  /** Deltas de 32 B: `{vertex_index, dpos, dnormal, pad}`. */
  deltas: Float32Array;
  totalDeltas: number;
  /** Canais com peso não nulo (o que o header do compute usa). */
  activeChannelCount: number;
  topologyHash: string;
  catalogFingerprint: string;
  staticRevision: number;
  baseGender: "Male" | "Female";
  meshUri: string;
  /** Valores de slider autorais do núcleo (`slider_id` → `value`). */
  morphValues: Map<string, number>;
}

/** Raised when a snapshot cannot be turned into viewport geometry. */
export class ViewportGeometryError extends Error {
  readonly code: string;
  constructor(code: string, detail: string) {
    super(`[ANIGO viewport geometry ${code}] ${detail}`);
    this.name = "ViewportGeometryError";
    this.code = code;
  }
}

/**
 * Valida a ordenação por `vertex_index` de cada canal.
 *
 * O compute canônico faz busca binária dentro do canal, então um canal fora de
 * ordem renderiza silenciosamente errado. É melhor falhar aqui.
 */
export function assertChannelsSorted(
  deltaWords: Uint32Array,
  channels: readonly { sliderId: string; startOffset: number; deltaCount: number }[]
): void {
  const wordsPerDelta = MORPH_DELTA_STRIDE_BYTES / 4;
  for (const channel of channels) {
    let previous = -1;
    for (let index = 0; index < channel.deltaCount; index++) {
      const word = (channel.startOffset + index) * wordsPerDelta;
      const vertexIndex = deltaWords[word];
      if (vertexIndex <= previous) {
        throw new ViewportGeometryError(
          "CHANNEL_NOT_SORTED",
          `canal '${channel.sliderId}' não está ordenado por vertex_index (posição ${index}: ${vertexIndex} ≤ ${previous})`
        );
      }
      previous = vertexIndex;
    }
  }
}

/** Registros de canal no layout `MorphChannel` do WGSL (16 B = 4 × u32/f32). */
export function packChannelRecords(channels: readonly ViewportMorphChannel[]): Float32Array {
  const words = new Uint32Array(channels.length * (MORPH_CHANNEL_STRIDE_BYTES / 4));
  const floats = new Float32Array(words.buffer);
  channels.forEach((channel, index) => {
    const base = index * (MORPH_CHANNEL_STRIDE_BYTES / 4);
    floats[base] = channel.weight;
    words[base + 1] = channel.startOffset;
    words[base + 2] = channel.deltaCount;
    words[base + 3] = 0;
  });
  return floats;
}

/**
 * Registros de canal (16 B) com os pesos efetivos do frame.
 *
 * O domínio de canais é sempre o do snapshot (157 sliders): sliders sem peso no
 * payload dinâmico recebem `0`, porque `peso = valor − default` — ausência no
 * snapshot significa "no default", nunca "não sei".
 */
export function packChannelRecordsWithWeights(
  channels: readonly ViewportMorphChannel[],
  weights: Map<string, number> | ReadonlyMap<string, number>
): Float32Array {
  return packChannelRecords(
    channels.map((channel) => ({
      ...channel,
      weight: weights.get(channel.sliderId) ?? 0,
    }))
  );
}

/**
 * Converte o snapshot em geometria de viewport.
 *
 * `null` quando o núcleo não mandou a parte estática (o cliente já tem a mesma
 * revisão) — nesse caso o viewport mantém os buffers atuais.
 */
export function viewportGeometryFromSnapshot(snapshot: CoreSnapshotWire): ViewportGeometry | null {
  const decoded = decodeCoreSnapshot(snapshot);
  if (!decoded.geometry) return null;
  return viewportGeometryFromDecoded(decoded.geometry, decoded.morphWeights, {
    staticRevision: decoded.staticRevision,
    morphValues: decoded.morphValues,
  });
}

/** Mesma conversão, a partir da geometria já decodificada. */
export function viewportGeometryFromDecoded(
  geometry: CoreGeometry,
  morphWeights: Map<string, number>,
  options: { staticRevision?: number; morphValues?: Map<string, number> } = {}
): ViewportGeometry {
  const expectedVertexBytes = geometry.vertexCount * VERTEX_STRIDE_BYTES;
  if (geometry.packedVertices.byteLength !== expectedVertexBytes) {
    throw new ViewportGeometryError(
      "BAD_VERTEX_COUNT",
      `vertex_count ${geometry.vertexCount} não bate com ${geometry.packedVertices.byteLength} bytes de vértice`
    );
  }
  if (geometry.indices.length !== geometry.indexCount) {
    throw new ViewportGeometryError(
      "BAD_INDEX_COUNT",
      `index_count ${geometry.indexCount} não bate com ${geometry.indices.length} índices`
    );
  }
  if (geometry.indices.length % 3 !== 0) {
    throw new ViewportGeometryError(
      "BAD_INDEX_COUNT",
      `${geometry.indices.length} índices não formam triângulos`
    );
  }

  const channels: ViewportMorphChannel[] = geometry.channels.map((descriptor) => {
    const weight = morphWeights.get(descriptor.slider_id) ?? 0;
    if (descriptor.start_offset + descriptor.delta_count > geometry.totalDeltas) {
      throw new ViewportGeometryError(
        "CHANNEL_OUT_OF_RANGE",
        `canal '${descriptor.slider_id}' aponta para deltas fora do buffer (${descriptor.start_offset}+${descriptor.delta_count} > ${geometry.totalDeltas})`
      );
    }
    return {
      target: descriptor.target,
      sliderId: descriptor.slider_id,
      weight,
      startOffset: descriptor.start_offset,
      deltaCount: descriptor.delta_count,
    };
  });

  assertChannelsSorted(geometry.deltaWords, channels);

  const active = channels.filter((channel) => channel.weight !== 0 && channel.deltaCount > 0);

  return {
    vertices: geometry.packedVertices,
    indices: geometry.indices,
    vertexCount: geometry.vertexCount,
    indexCount: geometry.indexCount,
    triangleCount: geometry.indexCount / 3,
    channels,
    channelRecords: packChannelRecords(channels),
    deltas: geometry.deltas,
    totalDeltas: geometry.totalDeltas,
    activeChannelCount: active.length,
    topologyHash: geometry.topologyHash,
    catalogFingerprint: geometry.catalogFingerprint,
    staticRevision: options.staticRevision ?? 0,
    baseGender: geometry.baseGender,
    meshUri: geometry.meshUri,
    morphValues: options.morphValues ?? new Map(),
  };
}

/**
 * Aplica os deltas **do núcleo** no CPU.
 *
 * Usado apenas onde o compute canônico não roda (WebGL2, que não tem compute
 * shaders). Não é uma segunda implementação de deformação: os deltas, os canais
 * e os pesos são exatamente os do snapshot — só o meio de acumulação difere.
 * A conta espelha o WGSL: `p += w·Δp`, `n = normalize(n + Σ w·Δn)`.
 */
export function applyDeltasCpu(
  geometry: ViewportGeometry,
  weights: Map<string, number> | ReadonlyMap<string, number>
): Float32Array {
  const floatsPerVertex = VERTEX_STRIDE_BYTES / 4;
  const out = new Float32Array(geometry.vertices);
  const deltas = geometry.deltas;
  for (const channel of geometry.channels) {
    const weight = weights.get(channel.sliderId) ?? 0;
    // Mesmo epsilon do compute canônico (`abs(ch.weight) > 1e-6`).
    if (Math.abs(weight) <= 1e-6 || channel.deltaCount === 0) continue;
    for (let index = 0; index < channel.deltaCount; index++) {
      const base = (channel.startOffset + index) * 8;
      const vertexIndex = new Uint32Array(deltas.buffer, deltas.byteOffset + base * 4, 1)[0];
      if (vertexIndex >= geometry.vertexCount) continue;
      const target = vertexIndex * floatsPerVertex;
      out[target] += weight * deltas[base + 1];
      out[target + 1] += weight * deltas[base + 2];
      out[target + 2] += weight * deltas[base + 3];
      out[target + 3] += weight * deltas[base + 4];
      out[target + 4] += weight * deltas[base + 5];
      out[target + 5] += weight * deltas[base + 6];
    }
  }
  // Normais normalizadas por vértice (mesma condição do compute: n² > 1e-12).
  for (let vertex = 0; vertex < geometry.vertexCount; vertex++) {
    const base = vertex * floatsPerVertex + 3;
    const squared = out[base] * out[base] + out[base + 1] * out[base + 1] + out[base + 2] * out[base + 2];
    if (squared > 1e-12) {
      const inverse = 1 / Math.sqrt(squared);
      out[base] *= inverse;
      out[base + 1] *= inverse;
      out[base + 2] *= inverse;
    }
  }
  return out;
}

/**
 * Assinatura estável do que está na GPU. O viewport compara a assinatura antes
 * de recriar buffers: subir a mesma geometria duas vezes é desperdício, e
 * ignorar uma mudança de topologia é bug.
 */
export function geometrySignature(geometry: ViewportGeometry): string {
  const weights = geometry.channels
    .filter((channel) => channel.weight !== 0)
    .map((channel) => `${channel.sliderId}=${channel.weight.toPrecision(7)}`)
    .join(",");
  return [
    geometry.topologyHash,
    geometry.catalogFingerprint,
    geometry.staticRevision,
    geometry.vertexCount,
    geometry.indexCount,
    geometry.totalDeltas,
    weights,
  ].join("|");
}

/** Erros de snapshot que o viewport deve reportar como snapshot inválido. */
export function isSnapshotError(error: unknown): boolean {
  return error instanceof SnapshotContractError || error instanceof ViewportGeometryError;
}
