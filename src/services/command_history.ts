/**
 * ANIGO — CommandHistoryService (P0 undo/redo: itens 1–3).
 *
 * The **only** history in the app: TypeScript owns no undo stack and no copy of
 * the project. It records *commands* — never snapshots — so undo/redo is defined
 * by the commands that were accepted, and the persisted session is a log that
 * can be replayed on top of a base state:
 *
 *   base state (project file / autosave) + command log  →  full session
 *
 * Invariants (mirrors `crates/anigo-core/src/command.rs`):
 *   - a rejected command **never** enters the history or the log (`push` throws);
 *   - the log has `sequence` 1..n and one entry per accepted command;
 *   - a new command invalidates the redo stack;
 *   - the depth is capped (`MAX_COMMAND_HISTORY`) exactly like `CommandHistory`.
 *
 * The core remains the authority: `push` only accepts commands it hands back
 * with a matching outcome (`apply` already validated them), and `verify` is the
 * read-only gate used before trusting a log that came from disk.
 */

import {
  COMMAND_KINDS,
  isCommandWire,
  type CommandKind,
  type CommandWire,
} from "../contracts/commands.v1";
import type { CommandOutcomeWire } from "../contracts/commands.v1";
import {
  COMMAND_LOG_VERSION,
  type CommandLogEntryWire,
  type CommandLogWire,
} from "../contracts/command_log.v1";

/** Maximum number of commands kept in the session history (mirrors Rust). */
export const MAX_COMMAND_HISTORY = 60;

/** Stable prefix used by the core for undo descriptions (`CommandOutcome`). */
export const UNDO_PREFIX = "Undo ";
export const REDO_PREFIX = "Redo ";

export interface CommandHistorySnapshot {
  /** Ids of the commands in stack order (index 0 = oldest). */
  sequence: number[];
  /** Description of the next undo, when any. */
  undo_description: string | null;
  /** Description of the next redo, when any. */
  redo_description: string | null;
  undo_depth: number;
  redo_depth: number;
  revision: number;
}

export type CommandHistoryErrorCode =
  | "not_a_command"
  | "rejected_by_core"
  | "unknown_kind"
  | "outcome_mismatch"
  | "corrupt_log"
  | "newer_log"
  | "out_of_order"
  | "empty_log";

export class CommandHistoryError extends Error {
  readonly recoverable = true;
  readonly code: CommandHistoryErrorCode;

  // NOTE: no parameter properties — the tests run under Node's strip-only
  // TypeScript mode (see tests/register.mjs).
  constructor(code: CommandHistoryErrorCode, message: string) {
    super(message);
    this.name = "CommandHistoryError";
    this.code = code;
  }
}

export function isKnownCommandKind(kind: unknown): kind is CommandKind {
  return typeof kind === "string" && (COMMAND_KINDS as readonly string[]).includes(kind);
}

/**
 * Structural check of one log entry. Used before replaying a log read from
 * disk, so a corrupt session can never reach the core half-parsed.
 */
export function isCommandLogEntry(value: unknown): value is CommandLogEntryWire {
  if (typeof value !== "object" || value === null) return false;
  const entry = value as Record<string, unknown>;
  if (typeof entry.sequence !== "number" || !Number.isInteger(entry.sequence) || entry.sequence < 1) return false;
  if (typeof entry.revision !== "number" || !Number.isInteger(entry.revision) || entry.revision < 0) return false;
  if (typeof entry.description !== "string" || entry.description.length === 0) return false;
  if (typeof entry.scope !== "string") return false;
  if (!isKnownCommandKind((entry.command as { kind?: unknown } | undefined)?.kind)) return false;
  return isCommandWire(entry.command);
}

/**
 * Parses a persisted command log (the format written by
 * `CommandHistory::export_log` in Rust) without a core round-trip.
 */
export function parseCommandLog(json: string | unknown): CommandLogWire {
  let raw: unknown = json;
  if (typeof json === "string") {
    try {
      raw = JSON.parse(json);
    } catch (error) {
      throw new CommandHistoryError("corrupt_log", `log de comandos não é JSON: ${(error as Error).message}`);
    }
  }
  if (typeof raw !== "object" || raw === null) {
    throw new CommandHistoryError("corrupt_log", "log de comandos precisa ser um objeto");
  }
  const log = raw as Record<string, unknown>;
  const version = log.version;
  if (typeof version !== "number" || !Number.isInteger(version) || version < 1) {
    throw new CommandHistoryError("corrupt_log", "log de comandos sem 'version' válida");
  }
  if (version > COMMAND_LOG_VERSION) {
    throw new CommandHistoryError(
      "newer_log",
      `log na versão ${version}, mas este build entende até a ${COMMAND_LOG_VERSION}`
    );
  }
  if (!Array.isArray(log.entries)) {
    throw new CommandHistoryError("corrupt_log", "log de comandos sem 'entries'");
  }
  const entries = log.entries;
  for (let index = 0; index < entries.length; index++) {
    if (!isCommandLogEntry(entries[index])) {
      throw new CommandHistoryError("corrupt_log", `entrada ${index} do log é inválida`);
    }
    const sequence = (entries[index] as CommandLogEntryWire).sequence;
    if (sequence !== index + 1) {
      throw new CommandHistoryError(
        "out_of_order",
        `entrada ${index} tem sequence ${sequence}, esperado ${index + 1}`
      );
    }
  }
  return { version, entries: entries as CommandLogEntryWire[] };
}

/**
 * Read-only validator used before replay. It answers "can the core accept this
 * log?" without mutating anything and without knowing the internals of the
 * command set.
 *
 * `verify(entry)` returns `true` when the command is still valid against the
 * state the core produced after the previous entries.
 */
export function verifyCommandLog(
  log: CommandLogWire,
  verify: (command: CommandWire, entry: CommandLogEntryWire) => boolean
): { ok: true; entries: CommandLogEntryWire[] } | { ok: false; index: number; entry: CommandLogEntryWire } {
  for (let index = 0; index < log.entries.length; index++) {
    const entry = log.entries[index];
    if (!verify(entry.command, entry)) {
      return { ok: false, index, entry };
    }
  }
  return { ok: true, entries: log.entries };
}

/** Depth/limit rules shared with the core. */
export function dropOldest<T>(items: T[], limit: number): T[] {
  if (items.length <= limit) return items;
  return items.slice(items.length - limit);
}

function assertClearable(entry: CommandLogEntryWire): void {
  if (!isCommandLogEntry(entry)) {
    throw new CommandHistoryError("not_a_command", "entrada de histórico inválida");
  }
}

/** Scopes defined by the command contract. */
const SCOPES: readonly string[] = [
  "base_geometry",
  "deformation",
  "shading",
  "camera",
  "presentation",
  "project",
];

/**
 * Checks that an outcome really describes an applied command for this history:
 * revision/sequence continuity and a scope the contract knows. A malformed
 * outcome means the caller is about to record something that was not applied —
 * the history stays untouched.
 */
function assertOutcome(outcome: CommandOutcomeWire | null | undefined, expectedSequence: number | null): CommandOutcomeWire {
  if (outcome === null || outcome === undefined) {
    throw new CommandHistoryError(
      "rejected_by_core",
      "o core não devolveu um outcome: o comando foi recusado e não será registrado"
    );
  }
  if (!Number.isInteger(outcome.revision) || outcome.revision < 0) {
    throw new CommandHistoryError("outcome_mismatch", `revision inválida no outcome: ${outcome.revision}`);
  }
  if (typeof outcome.description !== "string" || outcome.description.length === 0) {
    throw new CommandHistoryError("outcome_mismatch", "outcome sem descrição");
  }
  if (!SCOPES.includes(outcome.scope)) {
    throw new CommandHistoryError("outcome_mismatch", `escopo desconhecido no outcome: ${String(outcome.scope)}`);
  }
  if (expectedSequence !== null && outcome.sequence !== expectedSequence) {
    throw new CommandHistoryError(
      "out_of_order",
      `outcome com sequence ${outcome.sequence}, esperado ${expectedSequence}`
    );
  }
  return outcome;
}

export class CommandHistoryService {
  private entries: CommandLogEntryWire[] = [];
  private redo: CommandLogEntryWire[] = [];
  private nextSequence = 1;
  private currentRevision = 0;
  private limit: number;

  constructor(limit: number = MAX_COMMAND_HISTORY) {
    if (!Number.isInteger(limit) || limit < 1) {
      throw new CommandHistoryError("not_a_command", `limite de histórico inválido: ${limit}`);
    }
    this.limit = limit;
  }

  get undo_depth(): number {
    return this.entries.length;
  }

  get redo_depth(): number {
    return this.redo.length;
  }

  get revision(): number {
    return this.currentRevision;
  }

  get can_undo(): boolean {
    return this.entries.length > 0;
  }

  get can_redo(): boolean {
    return this.redo.length > 0;
  }

  /** Próximo número de sequência que o histórico atribuirá. */
  get next_sequence(): number {
    return this.nextSequence;
  }

  get limitValue(): number {
    return this.limit;
  }

  lastUndoDescription(): string | null {
    return this.entries.length === 0 ? null : this.entries[this.entries.length - 1].description;
  }

  lastRedoDescription(): string | null {
    return this.redo.length === 0 ? null : this.redo[this.redo.length - 1].description;
  }

  /**
   * Records a command **that the core already accepted** (item 3: only valid
   * commands are persisted). Anything else throws and leaves the history and the
   * log untouched — the same "rejected commands are not recorded" rule as Rust.
   */
  push(command: CommandWire, outcome: CommandOutcomeWire | null): CommandLogEntryWire {
    const kind: unknown = command?.kind;
    if (!isKnownCommandKind(kind)) {
      throw new CommandHistoryError("unknown_kind", `tipo de comando desconhecido: ${String(kind)}`);
    }
    if (!isCommandWire(command)) {
      throw new CommandHistoryError("not_a_command", `comando '${kind}' não respeita o contrato v1`);
    }
    const applied = assertOutcome(outcome, this.nextSequence);
    const entry: CommandLogEntryWire = {
      sequence: this.nextSequence,
      revision: applied.revision,
      description: applied.description,
      scope: applied.scope as CommandLogEntryWire["scope"],
      command,
    };
    assertClearable(entry);

    this.entries = dropOldest([...this.entries, entry], this.limit);
    this.nextSequence = entry.sequence + 1;
    this.currentRevision = applied.revision;
    // Item 2/3: a new accepted command invalidates the redo stack.
    this.redo = [];
    return entry;
  }

  /**
   * Coalescing rule (mirrors `HistoryService`'s 500 ms window): a continuous
   * change that landed on the entry already recorded **replaces** its command
   * instead of adding a new one, so one undo step is still one command — the
   * value that settled. In core mode the same rule applies by sending the
   * command at settle time.
   */
  replaceLast(command: CommandWire, description?: string): CommandLogEntryWire | null {
    const top = this.entries[this.entries.length - 1];
    if (!top) return null;
    const kind: unknown = command?.kind;
    if (!isKnownCommandKind(kind) || !isCommandWire(command)) {
      throw new CommandHistoryError("not_a_command", `comando '${String(kind)}' não respeita o contrato v1`);
    }
    const replaced: CommandLogEntryWire = {
      ...top,
      description: description ?? top.description,
      scope: top.scope,
      command,
    };
    assertClearable(replaced);
    this.entries = [...this.entries.slice(0, -1), replaced];
    return replaced;
  }

  /**
   * Moves one entry between the stacks after the core applied the inverse.
   * `describe` receives the direction so the caller can keep the same wording
   * the core uses (`Undo …` / `Redo …`).
   */
  applyUndo(outcome: CommandOutcomeWire): CommandLogEntryWire | null {
    const entry = this.entries.pop();
    if (!entry) return null;
    assertOutcome(outcome, null); // undo/redo advance the revision, not the sequence
    this.currentRevision = outcome.revision;
    this.redo.push({
      ...entry,
      revision: outcome.revision,
      description: stripPrefix(entry.description),
    });
    return entry;
  }

  applyRedo(outcome: CommandOutcomeWire): CommandLogEntryWire | null {
    const entry = this.redo.pop();
    if (!entry) return null;
    assertOutcome(outcome, null);
    this.currentRevision = outcome.revision;
    const restored: CommandLogEntryWire = {
      ...entry,
      revision: outcome.revision,
      description: stripPrefix(entry.description),
    };
    this.entries = dropOldest([...this.entries, restored], this.limit);
    return restored;
  }

  /** Clears everything — used when a new project is opened. */
  clear(): void {
    this.entries = [];
    this.redo = [];
    this.nextSequence = 1;
    this.currentRevision = 0;
  }

  snapshot(): CommandHistorySnapshot {
    return {
      sequence: this.entries.map((entry) => entry.sequence),
      undo_description: this.lastUndoDescription(),
      redo_description: this.lastRedoDescription(),
      undo_depth: this.undo_depth,
      redo_depth: this.redo_depth,
      revision: this.currentRevision,
    };
  }

  /**
   * The persisted session: the accepted commands, in order. `serialize()` is
   * what autosave/project files store, so a whole session can be rebuilt by
   * replaying the log over the base state.
   */
  exportLog(): CommandLogWire {
    return { version: COMMAND_LOG_VERSION, entries: [...this.entries] };
  }

  serialize(): string {
    return JSON.stringify(this.exportLog());
  }

  /**
   * Rebuilds the history from a log read from disk (autosave/session restore).
   * The redo stack is intentionally empty: a restored session is not "undone".
   */
  restoreLog(log: CommandLogWire): CommandLogWire {
    const parsed = parseCommandLog(log);
    this.entries = dropOldest([...parsed.entries], this.limit);
    this.redo = [];
    this.nextSequence = (this.entries[this.entries.length - 1]?.sequence ?? 0) + 1;
    this.currentRevision = this.entries[this.entries.length - 1]?.revision ?? 0;
    return this.exportLog();
  }
}

function stripPrefix(description: string): string {
  if (description.startsWith(UNDO_PREFIX)) return description.slice(UNDO_PREFIX.length);
  if (description.startsWith(REDO_PREFIX)) return description.slice(REDO_PREFIX.length);
  return description;
}
