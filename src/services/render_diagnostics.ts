import { diagnosticCodeSpecs } from "../contracts/render_contract.v1";

/**
 * ANIGO — diagnóstico de erro do renderer (P1 — Robustez, item 2).
 *
 * Antes desta camada, falhas do viewport acabavam em `console.warn` solto ou em
 * `catch (_) {}` (falha silenciosa): o usuário via um canvas preto e nada mais.
 * Aqui todo problema vira um **diagnóstico estruturado** — código estável,
 * severidade, contexto e contagem — que o shell pode mostrar na UI, mandar para
 * a telemetria do núcleo e usar em testes.
 *
 * Regras do contrato:
 * 1. código desconhecido é recusado (o catálogo abaixo é a fonte da verdade, e o
 *    teste de contrato falha se um `report` novo não estiver nele);
 * 2. repetições do mesmo código+mensagem não inflam a lista: incrementam `count`
 *    (o render loop pode chamar `report` a 60 fps);
 * 3. a lista é limitada (`capacity`) — o excedente é contabilizado em `dropped`,
 *    para a telemetria ser honesta sobre o que não coube.
 */

/** Severidade de um diagnóstico (a UI decide o que destacar). */
export type DiagnosticSeverity = "info" | "warning" | "error";

/**
 * Códigos estáveis de diagnóstico do renderer/viewport.
 *
 * A **severidade** não é declarada aqui: ela vem do render contract
 * (`diagnostics.codes`), que é o mesmo documento que o Rust lê. Assim um código
 * não pode ser aviso no viewport e erro no headless.
 */
export const DIAGNOSTIC_CODES = [
  /** WebGPU indisponível: caímos no fallback WebGL2. */
  "webgpu_unavailable",
  /** WebGL2 também falhou: não há backend para desenhar. */
  "backend_unavailable",
  /** O device WebGPU reportou erro (validation/uncaptured). */
  "gpu_device_error",
  /** Pipeline/estrutura canônica não pôde ser criada. */
  "pipeline_init_failed",
  /** Falha ao criar o compute canônico de morphs. */
  "compute_init_failed",
  /** Falha ao subir pesos de canal na GPU. */
  "channel_upload_failed",
  /** Contexto de canvas não pôde ser configurado (vsync/present). */
  "context_configure_failed",
  /** O swap chain não devolveu textura neste frame (frame pulado). */
  "frame_skipped",
  /** Snapshot do núcleo inválido/ilegível. */
  "snapshot_invalid",
  /** Snapshot do núcleo chegou fora de ordem e foi descartado. */
  "snapshot_stale",
  /** Geometria canônica indisponível (modo degradado, sem deformação). */
  "geometry_unavailable",
  /** Malha base (GLB) inválida antes de criar buffers. */
  "mesh_invalid",
  /** GLB não pôde ser carregado. */
  "model_load_failed",
  /** Geometria do snapshot reprovada na validação (strides/índices/ordem). */
  "geometry_rejected",
  /** Canal de morph pedido para um slider sem canal canônico. */
  "channel_missing",
  /** Erro inesperado tratado no caminho crítico. */
  "unexpected_error",
  /** O render contract divergiu do que o renderer espera. */
  "contract_drift",
  /** Nenhum adaptador/device disponível para renderizar. */
  "device_unavailable",
  /** O device WebGPU/wgpu foi perdido (driver crash, troca de GPU, suspensão). */
  "device_lost",
  /** O device foi recriado e o estado reapresentado (recuperação concluída). */
  "device_recreated",
  /** Shader de produção não compilou. */
  "shader_compile_failed",
  /** Buffer de GPU não pôde ser criado. */
  "buffer_creation_failed",
  /** Leitura de buffer da GPU (readback) falhou. */
  "readback_failed",
] as const;

export type DiagnosticCode = (typeof DIAGNOSTIC_CODES)[number];

/** Severidades declaradas no contrato (fonte da verdade). */
const CONTRACT_SEVERITIES: ReadonlyMap<string, DiagnosticSeverity> = new Map(
  diagnosticCodeSpecs().map((spec) => [spec.code, spec.severity])
);

/** Severidade de um código; `"error"` quando o contrato não declara o código. */
export function severityOf(code: DiagnosticCode): DiagnosticSeverity {
  return CONTRACT_SEVERITIES.get(code) ?? "error";
}

/**
 * Diferenças entre este catálogo e o do render contract.
 *
 * O teste de contrato exige lista vazia: um código usado no renderer e ausente
 * do contrato (ou o contrário) é drift, não estilo.
 */
export function diagnosticCatalogProblems(): string[] {
  const problems: string[] = [];
  const contractCodes = new Set(CONTRACT_SEVERITIES.keys());
  for (const code of DIAGNOSTIC_CODES) {
    if (!contractCodes.has(code)) {
      problems.push(`código '${code}' usado no renderer mas ausente do render contract`);
    }
  }
  const known = new Set<string>(DIAGNOSTIC_CODES);
  for (const code of contractCodes) {
    if (!known.has(code)) {
      problems.push(`código '${code}' declarado no contrato mas desconhecido do renderer`);
    }
  }
  return problems;
}

/** Um problema observado, agregado por código + mensagem. */
export interface RenderDiagnostic {
  code: DiagnosticCode;
  severity: DiagnosticSeverity;
  message: string;
  /** Detalhe técnico (mensagem de exceção, valor recusado). */
  detail: string | null;
  /** Contexto livre (preset, slider, revisão do snapshot…). */
  context: Record<string, string | number | boolean> | null;
  /** Quantas vezes o mesmo problema ocorreu. */
  count: number;
  /** `performance.now()` da primeira e da última ocorrência. */
  firstAt: number;
  lastAt: number;
}

/** Resumo pronto para telemetria/status bar. */
export interface DiagnosticsSummary {
  total: number;
  errors: number;
  warnings: number;
  codes: DiagnosticCode[];
  dropped: number;
  /** O renderer está degradado (há erro no caminho crítico). */
  degraded: boolean;
}

/** Interface mínima usada pelos consumidores (testável sem GPU). */
export interface DiagnosticsSink {
  report(
    code: DiagnosticCode,
    message: string,
    options?: { detail?: string; context?: Record<string, string | number | boolean> }
  ): RenderDiagnostic;
}

/** Erro de contrato: código fora do catálogo. */
export class DiagnosticContractError extends Error {
  readonly code: string;
  constructor(code: string) {
    super(
      `[ANIGO diagnostics] código desconhecido "${code}" — registre-o em DIAGNOSTIC_CODES antes de usá-lo`
    );
    this.name = "DiagnosticContractError";
    this.code = "unknown_diagnostic_code";
  }
}

/** `true` quando o código existe no catálogo (e no contrato). */
export function isDiagnosticCode(code: string): code is DiagnosticCode {
  return (DIAGNOSTIC_CODES as readonly string[]).includes(code) && CONTRACT_SEVERITIES.has(code);
}

/** Agregador de diagnósticos (um por renderer). */
export class RendererDiagnostics implements DiagnosticsSink {
  private entriesByKey = new Map<string, RenderDiagnostic>();
  private capacity: number;
  private droppedCount = 0;
  private clock: () => number;

  /** Notificado a cada ocorrência (mesmo quando agregada). */
  public onReport?: (diagnostic: RenderDiagnostic) => void;

  constructor(options: { capacity?: number; clock?: () => number } = {}) {
    this.capacity = Math.max(1, options.capacity ?? 64);
    this.clock = options.clock ?? (() => (typeof performance !== "undefined" ? performance.now() : Date.now()));
  }

  /** Registra um problema; valida o código contra o catálogo. */
  report(
    code: DiagnosticCode,
    message: string,
    options: { detail?: string; context?: Record<string, string | number | boolean> } = {}
  ): RenderDiagnostic {
    if (!isDiagnosticCode(code)) throw new DiagnosticContractError(String(code));
    const now = this.clock();
    const key = `${code}::${message}`;
    const existing = this.entriesByKey.get(key);
    if (existing) {
      existing.count += 1;
      existing.lastAt = now;
      if (options.detail) existing.detail = options.detail;
      if (options.context) existing.context = { ...(existing.context ?? {}), ...options.context };
      this.onReport?.(existing);
      return existing;
    }

    const diagnostic: RenderDiagnostic = {
      code,
      severity: severityOf(code),
      message,
      detail: options.detail ?? null,
      context: options.context ?? null,
      count: 1,
      firstAt: now,
      lastAt: now,
    };
    if (this.entriesByKey.size >= this.capacity) {
      this.droppedCount += 1;
      return diagnostic;
    }
    this.entriesByKey.set(key, diagnostic);
    this.onReport?.(diagnostic);
    return diagnostic;
  }

  /** Diagnósticos na ordem em que apareceram. */
  get entries(): readonly RenderDiagnostic[] {
    return [...this.entriesByKey.values()].sort((a, b) => a.firstAt - b.firstAt);
  }

  /** Diagnósticos de um código específico. */
  of(code: DiagnosticCode): readonly RenderDiagnostic[] {
    return this.entries.filter((entry) => entry.code === code);
  }

  /** Último diagnóstico registrado, se houver. */
  get last(): RenderDiagnostic | null {
    const entries = this.entries;
    return entries.length > 0 ? entries[entries.length - 1] : null;
  }

  get dropped(): number {
    return this.droppedCount;
  }

  /** Resumo para telemetria/status bar. */
  summary(): DiagnosticsSummary {
    const entries = this.entries;
    const errors = entries.filter((entry) => entry.severity === "error").length;
    const warnings = entries.filter((entry) => entry.severity === "warning").length;
    return {
      total: entries.length,
      errors,
      warnings,
      codes: entries.map((entry) => entry.code),
      dropped: this.droppedCount,
      degraded: errors > 0,
    };
  }

  clear(): void {
    this.entriesByKey.clear();
    this.droppedCount = 0;
  }

  /** Formato de log de uma linha (usado em `console.warn` quando não há UI). */
  static format(diagnostic: RenderDiagnostic): string {
    const context = diagnostic.context ? ` ${JSON.stringify(diagnostic.context)}` : "";
    const detail = diagnostic.detail ? ` — ${diagnostic.detail}` : "";
    return `[ANIGO ${diagnostic.code}] ${diagnostic.message}${detail}${context}`;
  }
}
