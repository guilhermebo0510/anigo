/**
 * ANIGO — Alinhamento entre o undo do UI e o log de comandos (P0 undo/redo).
 *
 * O app tem dois históricos que precisam andar em lockstep:
 *   - `HistoryService` (snapshots do UI) — restaura o estado visual do app;
 *   - `CommandHistoryService` (comandos) — é o que se persiste e se reproduz.
 *
 * Nem toda entrada do UI é um comando (ex.: trocas puramente visuais). Para que
 * `undo`/`redo` nunca fiquem fora de fase, `HistoryAlignment` guarda, na mesma
 * ordem, a marca de cada entrada: `command-backed` ou `UI-only`, além do comando
 * que a entrada representa.
 *
 * Regras:
 *   - `markAdded` é chamado quando o histórico do UI **criou** uma entrada;
 *   - `markCoalesced` é chamado quando a entrada criada foi **atualizada** por uma
 *     sequência contínua (drag de slider): o comando correspondente é substituído,
 *     não acrescentado — a entrada de undo continua sendo uma só;
 *   - `undo`/`redo` devolvem a marca que saiu do topo, para que o chamador mova o
 *     comando correspondente no `CommandHistoryService`.
 */

export type HistoryEntryOrigin = "command" | "ui";

export interface HistoryMarker<TCommand> {
  origin: HistoryEntryOrigin;
  /** Comando desta entrada (apenas quando `origin === "command"`). */
  command: TCommand | null;
}

export class HistoryAlignment<TCommand> {
  private undoStack: HistoryMarker<TCommand>[] = [];
  private redoStack: HistoryMarker<TCommand>[] = [];

  get undo_depth(): number {
    return this.undoStack.length;
  }

  get redo_depth(): number {
    return this.redoStack.length;
  }

  get can_undo(): boolean {
    return this.undoStack.length > 0;
  }

  get can_redo(): boolean {
    return this.redoStack.length > 0;
  }

  /** Uma entrada nova do histórico do UI. */
  markAdded(origin: HistoryEntryOrigin, command: TCommand | null = null): HistoryMarker<TCommand> {
    if (origin === "command" && command === null) {
      throw new Error("entrada 'command' precisa do comando correspondente");
    }
    const marker: HistoryMarker<TCommand> = { origin, command: origin === "command" ? command : null };
    this.undoStack.push(marker);
    this.redoStack = [];
    return marker;
  }

  /**
   * Atualização contínua (coalescida) da entrada do topo: o comando anterior é
   * substituído pelo novo valor que estabilizou.
   */
  markCoalesced(command: TCommand | null): boolean {
    const top = this.undoStack[this.undoStack.length - 1];
    if (!top) return false;
    if (top.origin !== "command" || command === null) return false;
    top.command = command;
    return true;
  }

  /** Entrada desfeita (movida para a pilha de redo). */
  undo(): HistoryMarker<TCommand> | null {
    const marker = this.undoStack.pop();
    if (!marker) return null;
    this.redoStack.push(marker);
    return marker;
  }

  /** Entrada refeita (volta para a pilha de undo). */
  redo(): HistoryMarker<TCommand> | null {
    const marker = this.redoStack.pop();
    if (!marker) return null;
    this.undoStack.push(marker);
    return marker;
  }

  clear(): void {
    this.undoStack = [];
    this.redoStack = [];
  }
}
