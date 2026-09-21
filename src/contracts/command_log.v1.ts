/**
 * ANIGO — Contrato do log de comandos v1 (P0 undo/redo).
 *
 * Espelho de `CommandLog`/`CommandLogEntry` em `crates/anigo-core/src/command.rs`
 * e do fixture congelado `contracts/fixtures/command_log_v1.json`.
 *
 * O log é o formato persistido da sessão: os comandos aceitos, em ordem, com a
 * revisão que cada um produziu. Replay = base + log, nunca um snapshot paralelo.
 */

import type { ChangeScope, CommandWire } from "./commands.v1";

/** Versão do wire format do log (mirrors Rust `COMMAND_LOG_VERSION`). */
export const COMMAND_LOG_VERSION = 1;

export interface CommandLogEntryWire {
  /** Número de sequência atribuído pelo histórico (1..n). */
  sequence: number;
  /** Revisão do projeto produzida por este comando. */
  revision: number;
  /** Descrição exibida no UI de undo. */
  description: string;
  /** Escopo da mudança (define rebuild estático, etc.). */
  scope: ChangeScope;
  /** O comando em si. */
  command: CommandWire;
}

export interface CommandLogWire {
  /** Versão do formato do log. */
  version: number;
  /** Comandos aceitos, do mais antigo para o mais novo. */
  entries: CommandLogEntryWire[];
}

/** Campos obrigatórios de uma entrada — usado pelos testes de contrato. */
export const COMMAND_LOG_ENTRY_FIELDS = [
  "sequence",
  "revision",
  "description",
  "scope",
  "command",
] as const satisfies ReadonlyArray<keyof CommandLogEntryWire>;
