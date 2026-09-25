/**
 * ANIGO — Escopo das mudanças e outcomes de preview (P0 undo/redo).
 *
 * `scopeOfCommand` espelha `Command::scope()` + `scope_priority()` do core: o
 * escopo diz o que precisa ser reconstruído (`requires_static_rebuild`) e é parte
 * do `CommandOutcome` que o app guarda no log.
 *
 * `previewOutcome` existe para o modo *preview* (sem core acessível): o app já
 * marcava esses envelopes como `source: "preview"` (ver `project_persistence`),
 * e aqui a mesma ideia vale para o log de comandos — comandos válidos, contagem
 * de revisão local. Quando o core está disponível, é o outcome dele que entra
 * (`CommandHistoryService.push` rejeita qualquer outcome fora de ordem).
 */

import type { ChangeScope, CommandOutcomeWire, CommandWire } from "../contracts/commands.v1";

/** Prioridade usada por `Command::Batch` para escolher o escopo mais forte. */
export const SCOPE_PRIORITY: Record<ChangeScope, number> = {
  base_geometry: 5,
  deformation: 4,
  shading: 3,
  camera: 2,
  presentation: 1,
  project: 0,
};

const SCOPE_BY_KIND: Record<string, ChangeScope> = {
  set_morph_value: "deformation",
  reset_morphs: "deformation",
  set_somatotype: "deformation",
  set_gender_dimorphism: "deformation",
  set_base_gender: "base_geometry",
  set_proportions: "base_geometry",
  set_node_mesh: "base_geometry",
  load_mesh_preset: "base_geometry",
  set_material_params: "shading",
  set_camera: "camera",
  orbit_camera: "camera",
  zoom_camera: "camera",
  pan_camera: "camera",
  set_light: "presentation",
  set_node_visibility: "presentation",
  // Issue #12: a topologia e a transformação dos nós são apresentação — a
  // geometria base (a malha canônica deformada) continua a mesma.
  add_node: "presentation",
  remove_node: "presentation",
  set_node_parent: "presentation",
  set_node_transform: "presentation",
  set_background_color: "presentation",
  set_render_settings: "presentation",
  rename_project: "project",
};

/** Escopo de um comando — igual a `Command::scope()` no Rust, inclusive no batch. */
export function scopeOfCommand(command: CommandWire): ChangeScope {
  if (command.kind === "batch") {
    if (command.commands.length === 0) return "project";
    return command.commands
      .map(scopeOfCommand)
      .reduce((strongest, scope) => (SCOPE_PRIORITY[scope] > SCOPE_PRIORITY[strongest] ? scope : strongest));
  }
  return SCOPE_BY_KIND[command.kind] ?? "project";
}

/** `true` quando o escopo exige reconstruir a geometria base (`ChangeScope`). */
export function requiresStaticRebuild(scope: ChangeScope): boolean {
  return scope === "base_geometry";
}

/** Ids tocados pelo comando — mesma ideia de `CommandOutcome::affected`. */
export function affectedTargets(command: CommandWire): string[] {
  const record = command as Record<string, unknown>;
  const found = ["target", "node_id", "material_id", "light_id"].filter(
    (key) => typeof record[key] === "string"
  );
  return found.map((key) => record[key] as string);
}

export interface PreviewOutcomeInput {
  command: CommandWire;
  description: string;
  /** Sequência que o histórico local atribuirá à entrada. */
  sequence: number;
  /** Revisão local após a operação. */
  revision: number;
  /** Profundidade do undo depois da operação. */
  undoDepth: number;
  /** Revisão da geometria estática (incrementa em `base_geometry`). */
  baseGeometryRevision?: number;
}

/**
 * Outcome local para o modo preview. Nunca é usado para "inventar" um comando:
 * o comando já foi validado por `buildCommand`, e o envelope persistido continua
 * marcado como `source: "preview"`.
 */
export function previewOutcome(input: PreviewOutcomeInput): CommandOutcomeWire {
  const scope = scopeOfCommand(input.command);
  const baseGeometryRevision =
    input.baseGeometryRevision ?? (requiresStaticRebuild(scope) ? input.revision : 0);
  return {
    sequence: input.sequence,
    revision: input.revision,
    base_geometry_revision: baseGeometryRevision,
    description: input.description,
    scope,
    affected: affectedTargets(input.command),
    can_undo: input.undoDepth > 0,
    can_redo: false,
    undo_depth: input.undoDepth,
    redo_depth: 0,
  };
}

/** Outcome de uma operação de undo/redo (a sequência não avança, a revisão sim). */
export function previewHistoryOutcome(options: {
  description: string;
  revision: number;
  undoDepth: number;
  redoDepth: number;
}): CommandOutcomeWire {
  return {
    sequence: 0,
    revision: options.revision,
    base_geometry_revision: 0,
    description: options.description,
    scope: "project",
    affected: [],
    can_undo: options.undoDepth > 0,
    can_redo: options.redoDepth > 0,
    undo_depth: options.undoDepth,
    redo_depth: options.redoDepth,
  };
}
