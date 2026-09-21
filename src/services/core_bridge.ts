/**
 * ANIGO — ponte com a sessão canônica do núcleo (P0 §7, itens 4–5).
 *
 * Regra da arquitetura que este módulo implementa (§2.2/§4.1): **o TypeScript
 * não deforma**. Ele envia comandos canônicos, recebe snapshots e os entrega ao
 * viewport. Não existem deltas, normais ou vértices autorais aqui — só o
 * transporte e a contabilidade de revisões.
 *
 * O transporte é injetável (`CoreInvoker`) para que o caminho completo possa ser
 * testado sem Tauri/GPU: os testes passam um `invoke` falso e verificam que o
 * cliente pede, decodifica e entrega exatamente o que o núcleo mandou.
 */

import {
  SNAPSHOT_FORMAT_VERSION,
  decodeCoreSnapshot,
  type CoreSnapshotWire,
} from "../contracts/core_snapshot.v1";
import { COMMAND_KINDS, type CommandOutcomeWire, type CommandWire } from "../contracts/commands.v1";

/** Assinatura de `invoke` do Tauri (injetável). */
export type CoreInvoker = (command: string, args?: Record<string, unknown>) => Promise<unknown>;

/** Códigos de falha da ponte (o UI decide o que fazer com cada um). */
export type CoreBridgeErrorCode =
  | "core_unavailable"
  | "invoke_failed"
  | "invalid_response"
  | "rejected_by_core"
  | "unsupported_version";

/** Falha explícita e recuperável: nenhum estado do editor é corrompido por ela. */
export class CoreBridgeError extends Error {
  readonly code: CoreBridgeErrorCode;
  readonly detail: string;
  constructor(code: CoreBridgeErrorCode, detail: string) {
    super(`[ANIGO core bridge ${code}] ${detail}`);
    this.name = "CoreBridgeError";
    this.code = code;
    this.detail = detail;
  }
}

const CORE_COMMANDS = {
  apply: "core_apply_command",
  undo: "core_undo_command",
  redo: "core_redo_command",
  snapshot: "core_snapshot",
  history: "core_history_state",
  loadDocument: "core_load_document",
  document: "core_document",
  deformedMesh: "core_deformed_mesh",
} as const;

/** Nome do comando Tauri de cada operação (contrato com `src-tauri/src/main.rs`). */
export function coreCommandName(operation: keyof typeof CORE_COMMANDS): string {
  return CORE_COMMANDS[operation];
}

/**
 * Resolve o `invoke` do Tauri quando o app roda no webview; `null` no browser
 * (dev server), onde não existe núcleo — nesse caso o viewport fica explicitamente
 * degradado em vez de deformar por conta própria.
 */
export async function resolveCoreInvoker(): Promise<CoreInvoker | null> {
  const globalScope = globalThis as { __TAURI_INTERNALS__?: unknown };
  if (typeof window === "undefined" || globalScope.__TAURI_INTERNALS__ === undefined) {
    return null;
  }
  try {
    const module: { invoke: CoreInvoker } = await import("@tauri-apps/api/core");
    return (command, args) => module.invoke(command, args);
  } catch {
    return null;
  }
}

/** Estado devolvido pelo núcleo em `core_history_state`. */
export interface CoreHistoryReport {
  revision: number;
  base_geometry_revision: number;
  dynamic_revision: number;
  static_revision: number;
  can_undo: boolean;
  can_redo: boolean;
  undo_depth: number;
  redo_depth: number;
  last_undo_description?: string | null;
  last_redo_description?: string | null;
  project_id?: string;
  project_name?: string;
  project_fingerprint?: number;
  project_schema_version?: number;
}

function asRecord(value: unknown): Record<string, unknown> | null {
  return typeof value === "object" && value !== null ? (value as Record<string, unknown>) : null;
}

/** Validates that a value looks like a `CoreSnapshot` v1 payload. */
export function isCoreSnapshot(value: unknown): value is CoreSnapshotWire {
  const record = asRecord(value);
  if (!record) return false;
  if (record.snapshot_version !== SNAPSHOT_FORMAT_VERSION) return false;
  return asRecord(record.dynamic) !== null;
}

/** Validates a `CommandOutcome` coming from the core. */
export function isCommandOutcome(value: unknown): value is CommandOutcomeWire {
  const record = asRecord(value);
  if (!record) return false;
  return (
    typeof record.sequence === "number" &&
    typeof record.revision === "number" &&
    typeof record.description === "string" &&
    typeof record.scope === "string"
  );
}

/** Validates a `core_history_state` payload. */
export function isCoreHistoryReport(value: unknown): value is CoreHistoryReport {
  const record = asRecord(value);
  if (!record) return false;
  return (
    typeof record.revision === "number" &&
    typeof record.static_revision === "number" &&
    typeof record.can_undo === "boolean" &&
    typeof record.can_redo === "boolean"
  );
}

/** Wrapper do transporte: valida a forma de tudo que sai/entra do núcleo. */
export class CoreTransport {
  private invokerRef: CoreInvoker | null;
  private lastError: string | null = null;

  constructor(invoker: CoreInvoker | null) {
    this.invokerRef = invoker;
  }

  /** Whether a core is reachable right now. */
  get available(): boolean {
    return this.invokerRef !== null;
  }

  /** Última falha de transporte (diagnóstico na status bar). */
  get error(): string | null {
    return this.lastError;
  }

  /** Invoker cru (usado pelos comandos de exportação, que têm payload próprio). */
  get invoker(): CoreInvoker | null {
    return this.invokerRef;
  }

  private async call(command: string, args?: Record<string, unknown>): Promise<unknown> {
    if (!this.invokerRef) {
      throw new CoreBridgeError("core_unavailable", `${command} requires the Rust core`);
    }
    try {
      const result = await this.invokerRef(command, args);
      this.lastError = null;
      return result;
    } catch (error) {
      this.lastError = error instanceof Error ? error.message : String(error);
      throw new CoreBridgeError("invoke_failed", `${command}: ${this.lastError}`);
    }
  }

  async applyCommand(command: CommandWire): Promise<CommandOutcomeWire> {
    if (!(COMMAND_KINDS as readonly string[]).includes(command.kind)) {
      throw new CoreBridgeError("rejected_by_core", `kind desconhecido: ${String(command.kind)}`);
    }
    const raw = await this.call(CORE_COMMANDS.apply, { command });
    if (!isCommandOutcome(raw)) {
      throw new CoreBridgeError("invalid_response", "core_apply_command não devolveu um outcome");
    }
    return raw;
  }

  async undo(): Promise<CommandOutcomeWire> {
    const raw = await this.call(CORE_COMMANDS.undo);
    if (!isCommandOutcome(raw)) {
      throw new CoreBridgeError("invalid_response", "core_undo_command não devolveu um outcome");
    }
    return raw;
  }

  async redo(): Promise<CommandOutcomeWire> {
    const raw = await this.call(CORE_COMMANDS.redo);
    if (!isCommandOutcome(raw)) {
      throw new CoreBridgeError("invalid_response", "core_redo_command não devolveu um outcome");
    }
    return raw;
  }

  async snapshot(includeStatic: boolean, clientStaticRevision: number | null): Promise<CoreSnapshotWire> {
    const raw = await this.call(CORE_COMMANDS.snapshot, {
      includeStatic,
      clientStaticRevision: clientStaticRevision ?? null,
    });
    if (!isCoreSnapshot(raw)) {
      throw new CoreBridgeError(
        "unsupported_version",
        "snapshot do núcleo não é v1 (snapshot_version/dynamic ausentes)"
      );
    }
    return raw;
  }

  async historyState(): Promise<CoreHistoryReport> {
    const raw = await this.call(CORE_COMMANDS.history);
    if (!isCoreHistoryReport(raw)) {
      throw new CoreBridgeError("invalid_response", "core_history_state inválido");
    }
    return raw;
  }

  async loadDocument(document: string): Promise<CoreHistoryReport | null> {
    const raw = await this.call(CORE_COMMANDS.loadDocument, { document });
    return isCoreHistoryReport(raw) ? raw : null;
  }

  async document(): Promise<string> {
    const raw = await this.call(CORE_COMMANDS.document);
    if (typeof raw !== "string") {
      throw new CoreBridgeError("invalid_response", "core_document não devolveu texto");
    }
    return raw;
  }
}

/** Snapshot já decodificado + revisões, como o viewport precisa. */
export interface CoreSnapshotDelivery {
  includeStatic: boolean;
  staticRevision: number;
  dynamicRevision: number;
  geometry: ReturnType<typeof decodeCoreSnapshot>["geometry"];
  morphWeights: Map<string, number>;
  morphValues: Map<string, number>;
  authority: ReturnType<typeof decodeCoreSnapshot>["authority"];
  coverage: ReturnType<typeof decodeCoreSnapshot>["coverage"];
  state: ReturnType<typeof decodeCoreSnapshot>["state"];
}

/**
 * Cliente da sessão: mantém a cache key da parte estática e só aceita snapshots
 * que avançam (revisão menor ou igual = resposta atrasada, descartada).
 */
export class CoreSessionClient {
  private transport: CoreTransport;
  private staticRevision: number | null = null;
  private dynamicRevision: number | null = null;
  private staleDrops = 0;

  constructor(invoker: CoreInvoker | null) {
    this.transport = new CoreTransport(invoker);
  }

  get available(): boolean {
    return this.transport.available;
  }

  get static_revision(): number | null {
    return this.staticRevision;
  }

  get dynamic_revision(): number | null {
    return this.dynamicRevision;
  }

  /** Snapshots descartados por chegarem fora de ordem (telemetria). */
  get dropped_snapshots(): number {
    return this.staleDrops;
  }

  get error(): string | null {
    return this.transport.error;
  }

  /**
   * P0 §8: o invoker cru do núcleo, para serviços que chamam comandos próprios
   * (exportação). Fica na sessão para que o App não monte um segundo transporte.
   */
  get invoker(): CoreInvoker | null {
    return this.transport.invoker;
  }

  async applyCommand(command: CommandWire): Promise<CommandOutcomeWire> {
    const outcome = await this.transport.applyCommand(command);
    this.dynamicRevision = outcome.revision;
    return outcome;
  }

  async undo(): Promise<CommandOutcomeWire> {
    const outcome = await this.transport.undo();
    this.dynamicRevision = outcome.revision;
    return outcome;
  }

  async redo(): Promise<CommandOutcomeWire> {
    const outcome = await this.transport.redo();
    this.dynamicRevision = outcome.revision;
    return outcome;
  }

  async historyState(): Promise<CoreHistoryReport> {
    return this.transport.historyState();
  }

  async loadDocument(document: string): Promise<CoreHistoryReport | null> {
    const report = await this.transport.loadDocument(document);
    this.staticRevision = null;
    this.dynamicRevision = null;
    return report;
  }

  /**
   * Pede um snapshot e o decodifica.
   *
   * `geometry: null` significa "a parte estática não mudou" — o viewport deve
   * manter os buffers atuais, não apagar nada.
   */
  async pullSnapshot(force: boolean = false): Promise<CoreSnapshotDelivery> {
    const includeStatic = force || this.staticRevision === null;
    const snapshot = await this.transport.snapshot(includeStatic, this.staticRevision);
    const decoded = decodeCoreSnapshot(snapshot);

    if (
      this.dynamicRevision !== null &&
      decoded.dynamicRevision < this.dynamicRevision &&
      decoded.geometry === null
    ) {
      this.staleDrops += 1;
      throw new CoreBridgeError(
        "invalid_response",
        `snapshot dinâmico fora de ordem (${decoded.dynamicRevision} < ${this.dynamicRevision})`
      );
    }

    this.staticRevision = decoded.staticRevision;
    this.dynamicRevision = decoded.dynamicRevision;
    return {
      includeStatic: decoded.geometry !== null,
      staticRevision: decoded.staticRevision,
      dynamicRevision: decoded.dynamicRevision,
      geometry: decoded.geometry,
      morphWeights: decoded.morphWeights,
      morphValues: decoded.morphValues,
      authority: decoded.authority,
      coverage: decoded.coverage,
      state: decoded.state,
    };
  }
}

/**
 * Peso de canal que o compute canônico espera para um valor de slider.
 *
 * É `valor − default` do catálogo — a mesma definição de
 * `DeformationInputs::weight_of` (Rust). Não é deformação: é a tradução de um
 * valor autoral para o peso do canal cujos deltas vieram do núcleo, e é o que
 * permite o slider responder em 60 fps sem ida-e-volta ao núcleo.
 */
export function channelWeightOf(value: number, catalogDefault: number): number {
  const weight = value - catalogDefault;
  return Number.isFinite(weight) ? weight : 0;
}
