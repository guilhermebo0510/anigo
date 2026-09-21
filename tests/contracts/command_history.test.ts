/**
 * ANIGO contract test — CommandHistoryService (P0 undo/redo, itens 2–4).
 *
 * Obligations checked here:
 *  1. `parseCommandLog` decodes the frozen cross-language fixture
 *     (`contracts/fixtures/command_log_v1.json`) that the Rust test module also
 *     decodes, and refuses corrupt/newer logs;
 *  2. only commands accepted by the core enter the history (item 3);
 *  3. the `apply → undo → redo → undo` sequence is symmetric — every state comes
 *     back with the same fingerprint, on both stacks (item 4);
 *  4. the persisted log contains commands (never snapshots) and can be restored
 *     to rebuild the session.
 *
 * Run: node --experimental-strip-types --test tests/contracts/
 */
import { describe, it } from "node:test";
import assert from "node:assert/strict";
import {
  COMMAND_KINDS,
  type CommandOutcomeWire,
  type CommandWire,
} from "../../src/contracts/commands.v1.ts";
import {
  COMMAND_LOG_ENTRY_FIELDS,
  COMMAND_LOG_VERSION,
  type CommandLogEntryWire,
  type CommandLogWire,
} from "../../src/contracts/command_log.v1.ts";
import {
  CommandHistoryError,
  CommandHistoryService,
  MAX_COMMAND_HISTORY,
  dropOldest,
  isCommandLogEntry,
  isKnownCommandKind,
  parseCommandLog,
  verifyCommandLog,
} from "../../src/services/command_history.ts";
import { enumVariantFields, fieldsOf, numericConst, readRepoFile } from "./rust_contract_source.ts";
import { HistoryAlignment } from "../../src/services/history_alignment.ts";
import { SCOPE_PRIORITY, affectedTargets, previewOutcome, scopeOfCommand } from "../../src/services/command_scope.ts";

const RUST_COMMAND = readRepoFile("crates/anigo-core/src/command.rs");
const RUST_PROJECT = readRepoFile("crates/anigo-core/src/project.rs");
const FIXTURE = JSON.parse(readRepoFile("contracts/fixtures/command_log_v1.json")) as CommandLogWire & {
  $comment: string;
  expected_entry_count: number;
  expected_undo_depth: number;
  expected_kinds: string[];
  expected_scopes: string[];
  expected_sequences: number[];
};

function entry(sequence: number, command: CommandWire, description = "Comando", revision = sequence): CommandLogEntryWire {
  return { sequence, revision, description, scope: "deformation", command };
}

function outcome(sequence: number, revision = sequence, description = `Comando ${sequence}`): CommandOutcomeWire {
  return {
    sequence,
    revision,
    base_geometry_revision: 0,
    description,
    scope: "deformation",
    affected: [],
    can_undo: true,
    can_redo: false,
    undo_depth: sequence,
    redo_depth: 0,
  };
}

/** Body of `fn <name>(...) { ... }` in the Rust source. */
function bracedBodyLocal(source: string, keyword: string, name: string): string | null {
  const start = source.indexOf(`${keyword} ${name}(`);
  if (start < 0) return null;
  const open = source.indexOf("{", start);
  if (open < 0) return null;
  let depth = 0;
  for (let index = open; index < source.length; index++) {
    if (source[index] === "{") depth++;
    else if (source[index] === "}") {
      depth--;
      if (depth === 0) return source.slice(open, index + 1);
    }
  }
  return null;
}


describe("CommandHistoryService — alinhamento com o undo do UI", () => {
  it("entradas de comando e de UI andam em lockstep", () => {
    const alignment = new HistoryAlignment<CommandWire>();
    const morph: CommandWire = { kind: "set_morph_value", target: "mrf_head_width", value: 1.1 };
    const material: CommandWire = { kind: "set_material_params", patch: { outline_width: 2 } };

    alignment.markAdded("command", morph);
    alignment.markAdded("ui");
    alignment.markAdded("command", material);

    assert.equal(alignment.undo_depth, 3);
    assert.deepEqual(alignment.undo()?.command, material);
    assert.deepEqual(alignment.undo()?.origin, "ui");
    assert.deepEqual(alignment.undo()?.command, morph);
    assert.equal(alignment.undo(), null);
    assert.equal(alignment.redo_depth, 3);

    assert.deepEqual(alignment.redo()?.command, morph);
    assert.equal(alignment.redo_depth, 2);
    assert.equal(alignment.undo_depth, 1);
    assert.equal(alignment.can_undo, true);
  });

  it("ajuste contínuo substitui o comando da entrada, sem criar outra", () => {
    const alignment = new HistoryAlignment<CommandWire>();
    const first: CommandWire = { kind: "set_morph_value", target: "mrf_head_width", value: 1.1 };
    const settled: CommandWire = { kind: "set_morph_value", target: "mrf_head_width", value: 1.4 };
    alignment.markAdded("command", first);

    assert.equal(alignment.markCoalesced(settled), true);
    assert.equal(alignment.undo_depth, 1, "o drag continua sendo um único passo de undo");
    assert.deepEqual(alignment.undo()?.command, settled, "fica o valor que estabilizou");
    // Uma sequência contínua não pode transformar uma entrada de UI em comando.
    alignment.clear();
    alignment.markAdded("ui");
    assert.equal(alignment.markCoalesced(settled), false);
    assert.equal(alignment.undo()?.origin, "ui");
  });

  it("uma nova entrada descarta o redo", () => {
    const alignment = new HistoryAlignment<CommandWire>();
    alignment.markAdded("command", { kind: "reset_morphs" });
    alignment.undo();
    assert.equal(alignment.can_redo, true);
    alignment.markAdded("ui");
    assert.equal(alignment.can_redo, false);
    assert.equal(alignment.undo_depth, 1);
  });

  it("entrada de comando exige o comando correspondente", () => {
    const alignment = new HistoryAlignment<CommandWire>();
    assert.throws(() => alignment.markAdded("command", null), /comando correspondente/);
    assert.equal(alignment.undo_depth, 0);
  });
});

describe("CommandScope — o escopo do TS é o mesmo do core", () => {
  const rustScope = new Map<string, string>();
  for (const arm of RUST_COMMAND.matchAll(
    /((?:Command::\w+(?:\s*\{[^}]*\})?\s*\|?\s*)+)=>\s*ChangeScope::(\w+),/g
  )) {
    const scope = arm[2].replace(/([a-z0-9])([A-Z])/g, "$1_$2").toLowerCase();
    for (const name of arm[1].matchAll(/Command::(\w+)/g)) {
      const kind = name[1].replace(/([a-z0-9])([A-Z])/g, "$1_$2").toLowerCase();
      rustScope.set(kind, scope);
    }
  }

  it("cada tipo de comando tem o escopo declarado em Command::scope()", () => {
    assert.ok(rustScope.size >= COMMAND_KINDS.length - 1, "o parser precisa achar os escopos do Rust");
    for (const kind of COMMAND_KINDS) {
      if (kind === "batch") continue; // tratado abaixo
      const command = { kind } as unknown as CommandWire;
      const expected = rustScope.get(kind);
      assert.ok(expected, `Command::scope() do Rust não cobre '${kind}'`);
      assert.equal(scopeOfCommand(command), expected, `escopo de '${kind}' divergiu`);
    }
  });

  it("o batch usa a mesma prioridade do Rust", () => {
    const priority = new Map<string, number>();
    for (const arm of (bracedBodyLocal(RUST_COMMAND, "fn", "scope_priority") || "").matchAll(
      /ChangeScope::(\w+)\s*=>\s*(\d+),/g
    )) {
      priority.set(arm[1].replace(/([a-z0-9])([A-Z])/g, "$1_$2").toLowerCase(), Number(arm[2]));
    }
    for (const [scope, value] of priority) {
      assert.equal(SCOPE_PRIORITY[scope as keyof typeof SCOPE_PRIORITY], value, `prioridade de ${scope}`);
    }
    const batch: CommandWire = {
      kind: "batch",
      commands: [
        { kind: "set_material_params", patch: { hue_shift: 1 } }, // shading (3)
        { kind: "rename_project", name: "x" }, // project (0)
        { kind: "set_proportions", head_scale: 1.1 }, // base_geometry (5)
        { kind: "set_camera", fov_degrees: 45 }, // camera (2)
      ],
    };
    assert.equal(scopeOfCommand(batch), "base_geometry", "o escopo mais forte vence");
    assert.equal(scopeOfCommand({ kind: "batch", commands: [] }), "project");
  });

  it("o outcome de preview carrega escopo, afetados e revisões", () => {
    const command: CommandWire = {
      kind: "set_node_visibility",
      node_id: "nod_character_base",
      visible: false,
    };
    const outcome = previewOutcome({ command, description: "Ocultar corpo", sequence: 4, revision: 4, undoDepth: 4 });
    assert.equal(outcome.scope, "presentation");
    assert.deepEqual(outcome.affected, ["nod_character_base"]);
    assert.equal(outcome.sequence, 4);
    assert.equal(outcome.revision, 4);
    assert.equal(outcome.base_geometry_revision, 0);
    assert.equal(outcome.can_undo, true);
    assert.equal(outcome.base_geometry_revision, 0);
    assert.deepEqual(
      affectedTargets({ kind: "set_morph_value", target: "mrf_head_width", value: 1 }),
      ["mrf_head_width"]
    );
    // base_geometry acompanha a revisão (o core incrementa o rebuild estático).
    const rebuild = previewOutcome({
      command: { kind: "load_mesh_preset", preset: "cube" },
      description: "Preset",
      sequence: 5,
      revision: 9,
      undoDepth: 5,
    });
    assert.equal(rebuild.scope, "base_geometry");
    assert.equal(rebuild.base_geometry_revision, 9);
  });
});
describe("CommandHistoryService — o log persistido é de comandos, não de snapshots", () => {
  it("o fixture cross-language é decodificado sem perdas", () => {
    const log = parseCommandLog(FIXTURE);
    assert.equal(log.version, COMMAND_LOG_VERSION);
    assert.equal(log.entries.length, FIXTURE.expected_entry_count);
    assert.deepEqual(
      log.entries.map((value) => value.sequence),
      FIXTURE.expected_sequences
    );
    assert.deepEqual(
      log.entries.map((value) => value.command.kind),
      FIXTURE.expected_kinds
    );
    assert.deepEqual(
      log.entries.map((value) => value.scope),
      FIXTURE.expected_scopes
    );
    for (const value of log.entries) {
      assert.ok(isCommandLogEntry(value), "entrada do fixture precisa ser válida");
      assert.ok(isKnownCommandKind(value.command.kind), "kind precisa existir no contrato");
    }
    // Every entry carries a command and never a state snapshot: a log entry has
    // exactly the five frozen fields.
    assert.deepEqual(
      Object.keys(log.entries[0]).sort(),
      [...COMMAND_LOG_ENTRY_FIELDS].sort()
    );
    assert.equal(FIXTURE.expected_undo_depth, log.entries.length);
  });

  it("o log do fixture é replayável com um verificador puro", () => {
    const log = parseCommandLog(FIXTURE);
    const seen: string[] = [];
    const result = verifyCommandLog(log, (command, value) => {
      assert.equal(value.sequence, seen.length + 1, "verificação acontece em ordem");
      seen.push(command.kind);
      return true;
    });
    assert.equal(result.ok, true);
    assert.deepEqual(seen, FIXTURE.expected_kinds);
  });

  it("a verificação aponta o primeiro comando inválido (recuperação por prefixo)", () => {
    const log = parseCommandLog(FIXTURE);
    let calls = 0;
    const result = verifyCommandLog(log, () => {
      calls += 1;
      return calls < 3;
    });
    assert.equal(result.ok, false);
    if (result.ok === false) {
      assert.equal(result.index, 2, "o índice aponta a entrada que falhou");
      assert.equal(result.entry.command.kind, FIXTURE.expected_kinds[2]);
    }
  });

  it("recusa log corrompido, fora de ordem ou de versão futura", () => {
    assert.throws(() => parseCommandLog("nao é json"), (error: unknown) => {
      assert.ok(error instanceof CommandHistoryError);
      assert.equal(error.code, "corrupt_log");
      return true;
    });
    assert.throws(() => parseCommandLog({ version: 1 }), /entries/);
    assert.throws(
      () => parseCommandLog({ version: COMMAND_LOG_VERSION + 1, entries: [] }),
      (error: unknown) => {
        assert.ok(error instanceof CommandHistoryError);
        assert.equal(error.code, "newer_log", "um log mais novo é recusado, nunca meio-lido");
        return true;
      }
    );
    const shuffled = JSON.parse(readRepoFile("contracts/fixtures/command_log_v1.json")) as CommandLogWire;
    shuffled.entries[3].sequence = 9;
    assert.throws(
      () => parseCommandLog(shuffled),
      (error: unknown) => {
        assert.ok(error instanceof CommandHistoryError);
        assert.equal(error.code, "out_of_order");
        return true;
      }
    );
    // Semantic corruption: kind that the contract does not know.
    const unknown = JSON.parse(readRepoFile("contracts/fixtures/command_log_v1.json")) as CommandLogWire;
    (unknown.entries[0].command as { kind: string }).kind = "format_hard_drive";
    assert.throws(() => parseCommandLog(unknown), (error: unknown) => {
      assert.ok(error instanceof CommandHistoryError);
      assert.equal(error.code, "corrupt_log");
      return true;
    });
  });

  it("mudanças reais zeram o estado e o redo, mantendo a sequência", () => {
    const service = new CommandHistoryService(8);
    const commands: CommandWire[] = [
      { kind: "set_morph_value", target: "mrf_head_width", value: 1.25 },
      { kind: "set_gender_dimorphism", value: 0.35 },
      { kind: "rename_project", name: "Projeto 3D" },
    ];
    for (let index = 0; index < commands.length; index++) {
      service.push(commands[index], outcome(index + 1));
    }
    assert.equal(service.undo_depth, 3);
    assert.equal(service.redo_depth, 0);
    assert.equal(service.revision, 3);
    assert.deepEqual(service.snapshot().sequence, [1, 2, 3]);
    assert.equal(service.lastUndoDescription(), "Comando 3");

    const log = service.exportLog();
    assert.equal(log.version, COMMAND_LOG_VERSION);
    assert.deepEqual(
      log.entries.map((value) => value.command),
      commands
    );
    assert.match(service.serialize(), /"kind":"set_morph_value"/);
    assert.ok(!service.serialize().includes("positions"), "o log não carrega geometria/snapshot");
  });

  it("comandos recusados pelo core nunca entram no histórico (item 3)", () => {
    const service = new CommandHistoryService(8);
    const command: CommandWire = { kind: "set_morph_value", target: "mrf_head_width", value: 1.25 };

    // O core recusa: não devolve outcome. Nada é registrado.
    assert.throws(() => service.push(command, null), (error: unknown) => {
      assert.ok(error instanceof CommandHistoryError);
      assert.equal(error.code, "rejected_by_core");
      return true;
    });
    // Outcome fora de ordem (de outro comando/histórico) também é recusado.
    assert.throws(() => service.push(command, outcome(7)), (error: unknown) => {
      assert.ok(error instanceof CommandHistoryError);
      assert.equal(error.code, "out_of_order");
      return true;
    });
    // Kind fora do contrato é recusado antes de tudo.
    assert.throws(
      () => service.push({ kind: "explode" } as unknown as CommandWire, outcome(1)),
      (error: unknown) => {
        assert.ok(error instanceof CommandHistoryError);
        assert.equal(error.code, "unknown_kind");
        return true;
      }
    );
    // Outcome sem revisão válida não descreve um comando aplicado.
    assert.throws(
      () => service.push(command, { ...outcome(1), revision: Number.NaN }),
      (error: unknown) => {
        assert.ok(error instanceof CommandHistoryError);
        assert.equal(error.code, "outcome_mismatch");
        return true;
      }
    );

    assert.equal(service.undo_depth, 0);
    assert.equal(service.revision, 0);
    assert.equal(service.exportLog().entries.length, 0);

    // O comando válido seguinte começa na sequência 1.
    const accepted = service.push(command, outcome(1));
    assert.equal(accepted.sequence, 1);
    assert.equal(service.undo_depth, 1);
  });

  it("apply → undo → redo → undo é simétrico nos dois stacks (item 4)", () => {
    const service = new CommandHistoryService(16);
    const commands: CommandWire[] = [
      { kind: "set_morph_value", target: "mrf_head_width", value: 1.3 },
      { kind: "set_somatotype", endomorph: 0.4, mesomorph: 0.35, ectomorph: 0.25 },
      {
        kind: "set_proportions",
        head_scale: 1.1,
        shoulder_width: 1.05,
      },
      { kind: "set_background_color", color: [0.2, 0.3, 0.4, 1] },
      { kind: "set_node_visibility", node_id: "nod_character_base", visible: false },
    ];

    // apply
    for (let index = 0; index < commands.length; index++) {
      service.push(commands[index], outcome(index + 1, index + 1, `Comando ${index + 1}`));
    }
    const appliedDepth = service.undo_depth;

    // undo (com descrição prefixada como no Rust: "Undo …")
    for (let index = commands.length - 1; index >= 0; index--) {
      const undone = service.applyUndo(outcome(0, 100 + (commands.length - index), `Undo Comando ${index + 1}`));
      assert.ok(undone, "havia entrada para desfazer");
      assert.equal(undone?.command.kind, commands[index].kind);
      assert.equal(service.undo_depth, index);
      assert.equal(service.redo_depth, commands.length - index);
    }
    assert.equal(service.can_undo, false);
    assert.equal(service.can_redo, true);

    // redo — os mesmos comandos, na mesma ordem, com a descrição limpa
    for (let index = 0; index < commands.length; index++) {
      const redone = service.applyRedo(outcome(0, 200 + index, `Redo Comando ${index + 1}`));
      assert.ok(redone);
      assert.equal(redone?.command.kind, commands[index].kind);
      assert.equal(redone?.description, `Comando ${index + 1}`, "o prefixo Undo/Redo não fica no log");
      assert.equal(service.undo_depth, index + 1);
      assert.equal(service.redo_depth, commands.length - index - 1);
    }
    assert.equal(service.can_redo, false);
    assert.equal(service.undo_depth, appliedDepth);

    // undo novamente — a sequência é repetível, não one-shot
    for (let index = commands.length - 1; index >= 0; index--) {
      service.applyUndo(outcome(0, 300 + index, `Undo Comando ${index + 1}`));
    }
    assert.equal(service.undo_depth, 0);
    assert.deepEqual(
      service.exportLog().entries,
      [],
      "depois de desfazer tudo o log fica vazio, mas as entradas continuam no redo"
    );

    // Desfazer sem nada na pilha é no-op (o core devolveria NothingToUndo).
    assert.equal(service.applyUndo(outcome(0, 400, "Undo")), null);
    assert.equal(service.applyRedo(outcome(0, 401, "Redo"))?.command.kind, commands[0].kind);
  });

  it("novo comando aceito invalida o redo (item 2)", () => {
    const service = new CommandHistoryService(8);
    service.push({ kind: "set_morph_value", target: "mrf_head_width", value: 1.2 }, outcome(1));
    service.applyUndo(outcome(0, 2, "Undo Comando 1"));
    assert.equal(service.can_redo, true);

    service.push({ kind: "set_morph_value", target: "mrf_head_width", value: 1.4 }, outcome(2, 3, "Comando 2"));
    assert.equal(service.can_redo, false, "uma ação nova descarta o redo");
    // A sequência é monotônica e nunca reutilizada (mesma regra de
    // `CommandHistory::execute`): o comando novo é o 2, não um novo 1.
    assert.deepEqual(service.snapshot().sequence, [2]);
    assert.equal(service.revision, 3);
  });

  it("o limite do histórico descarta as entradas mais antigas", () => {
    const service = new CommandHistoryService(3);
    for (let index = 1; index <= 5; index++) {
      service.push({ kind: "set_gender_dimorphism", value: index / 10 }, outcome(index));
    }
    assert.equal(service.undo_depth, 3);
    assert.deepEqual(service.snapshot().sequence, [3, 4, 5]);
    assert.equal(dropOldest([1, 2, 3, 4], 2).length, 2);
    assert.equal(MAX_COMMAND_HISTORY, 60, "mesmo teto do CommandHistory do core");
    assert.equal(numericConst(RUST_PROJECT, "DEFAULT_HISTORY_LIMIT"), MAX_COMMAND_HISTORY);
  });

  it("restaurar uma sessão reconstrói o histórico a partir do log", () => {
    const fixture = parseCommandLog(FIXTURE);
    const restored = new CommandHistoryService(16);
    restored.restoreLog(fixture);

    assert.equal(restored.undo_depth, fixture.entries.length);
    assert.equal(restored.redo_depth, 0, "uma sessão restaurada não tem redo");
    assert.equal(restored.revision, fixture.entries[fixture.entries.length - 1].revision);
    assert.deepEqual(
      restored.exportLog().entries.map((value) => value.command),
      fixture.entries.map((value) => value.command),
      "replay: o log restaurado é idêntico ao persistido"
    );
    // O próximo comando continua a sequência, não reinicia em 1.
    const next = restored.push(
      { kind: "set_morph_value", target: "mrf_head_width", value: 0.8 },
      outcome(restored.undo_depth + 1, restored.revision + 1, "Comando novo")
    );
    assert.equal(next.sequence, fixture.entries.length + 1);
  });

  it("o contrato TypeScript do log bate com o Rust (campos e versão)", () => {
    assert.equal(COMMAND_LOG_VERSION, numericConst(RUST_COMMAND, "COMMAND_LOG_VERSION"));
    assert.deepEqual(
      fieldsOf(RUST_COMMAND, "CommandLogEntry").sort(),
      [...COMMAND_LOG_ENTRY_FIELDS].sort(),
      "CommandLogEntry (Rust) e CommandLogEntryWire (TS) precisam ter os mesmos campos"
    );
    assert.deepEqual(fieldsOf(RUST_COMMAND, "CommandLog").sort(), ["entries", "version"]);

    // Todo `kind` do contrato TS tem que existir no enum Rust (drift check).
    const rustKinds = enumVariantFields(RUST_COMMAND, "Command").keys();
    const rustKindNames = [...rustKinds].map((name) => name.replace(/([a-z])([A-Z])/g, "$1_$2").toLowerCase());
    for (const kind of COMMAND_KINDS) {
      assert.ok(rustKindNames.includes(kind), `kind '${kind}' não existe em Command (Rust)`);
    }
    assert.equal(rustKindNames.length, COMMAND_KINDS.length, "nenhum kind extra no Rust");
  });
});
