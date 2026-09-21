/**
 * ANIGO — Intent → Command (P0 undo/redo item 1: "converter alterações em comandos").
 *
 * The UI never mutates persistent state directly (ARQUITETURA §2.5). Every edit
 * is turned into one of the commands of `src/contracts/commands.v1.ts` and sent
 * to the Rust core, which owns the data, the validation and the history.
 *
 * This module is the *conversion + pre-flight* layer:
 *   - it is pure (no DOM, no state, no I/O);
 *   - it refuses what the core refuses, with the same vocabulary
 *     (`CommandError::{UnknownTarget, InvalidValue, NonFinite, NoOp}`);
 *   - it never invents values: ids are derived with the frozen rules
 *     (`mrf_<slider>` for morphs, `ast_<fnv1a64(uri)>` for assets).
 *
 * Rejecting locally is a UX optimization, not an authority: the core validates
 * again, and a command that reaches it with a bad value is still rejected.
 */

import {
  COMMAND_KINDS,
  type CommandWire,
  type MaterialPatchWire,
  type MeshPresetWire,
  type TonemapOperatorWire,
} from "../contracts/commands.v1";
import {
  ID_PREFIXES,
  isValidStableId,
  morphIdForSlider,
} from "../contracts/project_state.v1";
import { assetIdForUri } from "./project_persistence";
import { CANONICAL_SLIDERS, type MorphSlider } from "./morph_catalog";

/** Why an intent could not become a command (mirrors `CommandError`). */
export type CommandBuildErrorCode =
  | "unknown_target"
  | "invalid_value"
  | "non_finite"
  | "no_op"
  | "empty_batch";

export interface CommandBuildFailure {
  ok: false;
  code: CommandBuildErrorCode;
  /** Field/entity the failure refers to, when known. */
  field?: string;
  reason: string;
}

export interface CommandBuildSuccess {
  ok: true;
  command: CommandWire;
  /**
   * `true` when the intent carries an id the UI cannot check locally (a node or
   * material created by the core): the command is still built, the core decides.
   */
  unverifiedTarget?: boolean;
}

export type CommandBuildResult = CommandBuildSuccess | CommandBuildFailure;

/** Tolerance used by the core for "already has this value" (`1e-6`). */
export const NO_OP_EPSILON = 1e-6;

function fail(code: CommandBuildErrorCode, reason: string, field?: string): CommandBuildFailure {
  return { ok: false, code, reason, field };
}

function finite(value: unknown, field: string): CommandBuildFailure | null {
  if (typeof value !== "number" || !Number.isFinite(value)) {
    return fail("non_finite", `'${field}' precisa ser um número finito`, field);
  }
  return null;
}

function inRange(value: number, min: number, max: number, field: string): CommandBuildFailure | null {
  if (value < min || value > max) {
    return fail("invalid_value", `'${field}' precisa estar em [${min}, ${max}], recebido ${value}`, field);
  }
  return null;
}

/** Catalog entry of a morph slider (the UI's view of the canonical catalog). */
export function sliderOf(sliderId: string): MorphSlider | undefined {
  return CANONICAL_SLIDERS.find((slider) => slider.id === sliderId);
}

/** Identity helper used by the tests to prove the vocabulary is complete. */
export const BUILT_COMMAND_KINDS = COMMAND_KINDS;

// ---------------------------------------------------------------------------
// Intents
// ---------------------------------------------------------------------------

export interface ProportionsIntent {
  head_scale?: number;
  head_ratio?: number;
  shoulder_width?: number;
  leg_length?: number;
  arm_length?: number;
  neck_length?: number;
  torso_length?: number;
  height_overall?: number;
}

export interface LightIntent {
  light_id?: string;
  direction?: [number, number, number];
  color?: [number, number, number];
  intensity?: number;
  shadow_color?: [number, number, number];
  ambient_intensity?: number;
  shadow_saturation?: number;
  ambient_sky?: [number, number, number];
  ambient_ground?: [number, number, number];
}

export interface CameraIntent {
  eye?: [number, number, number];
  target?: [number, number, number];
  up?: [number, number, number];
  fov_degrees?: number;
}

export type CommandIntent =
  | { kind: "morph"; slider_id: string; value: number; /** current value, when the UI knows it */ current_value?: number }
  | { kind: "reset_morphs"; /** number of overrides currently set */ override_count?: number }
  | { kind: "base_gender"; gender: "male" | "female"; current_gender?: "male" | "female" }
  | {
      kind: "somatotype";
      endo: number;
      meso: number;
      ecto: number;
      current?: { endo: number; meso: number; ecto: number };
    }
  | { kind: "gender_dimorphism"; value: number; current_value?: number }
  | { kind: "proportions"; patch: ProportionsIntent }
  | ({ kind: "camera" } & CameraIntent)
  | { kind: "orbit"; azimuth: number; elevation: number }
  | { kind: "zoom"; factor: number; current_factor?: number }
  | { kind: "pan"; dx: number; dy: number }
  | { kind: "light"; patch: LightIntent }
  | { kind: "material"; patch: MaterialPatchWire; material_id?: string }
  | { kind: "node_visibility"; node_id: string; visible: boolean; current_visible?: boolean }
  | { kind: "node_mesh"; node_id: string; mesh_uri?: string | null; primitive_index?: number }
  | { kind: "preset"; preset: MeshPresetWire }
  | { kind: "background_color"; color: [number, number, number, number]; current_color?: [number, number, number, number] }
  | { kind: "render_settings"; msaa_samples?: number; tonemap?: TonemapOperatorWire }
  | { kind: "rename_project"; name: string; current_name?: string }
  | { kind: "batch"; intents: CommandIntent[] };

// ---------------------------------------------------------------------------
// Builders
// ---------------------------------------------------------------------------

const MSAA_SAMPLES = [1, 2, 4, 8, 16] as const;
const BACKGROUND_KEYS: Array<keyof LightIntent> = [
  "direction",
  "color",
  "intensity",
  "shadow_color",
  "ambient_intensity",
  "shadow_saturation",
  "ambient_sky",
  "ambient_ground",
];
const PROPORTION_KEYS: Array<keyof ProportionsIntent> = [
  "head_scale",
  "head_ratio",
  "shoulder_width",
  "leg_length",
  "arm_length",
  "neck_length",
  "torso_length",
  "height_overall",
];

function buildVec3(value: unknown, field: string): CommandBuildFailure | null {
  if (value === undefined) return null;
  if (!Array.isArray(value) || value.length !== 3) {
    return fail("invalid_value", `'${field}' precisa ser um vetor de 3 números`, field);
  }
  for (let axis = 0; axis < 3; axis++) {
    const problem = finite(value[axis], `${field}[${axis}]`);
    if (problem) return problem;
  }
  return null;
}

/**
 * Converts one UI intent into a validated command.
 *
 * Returns `{ ok: false }` for anything the core would reject, so an invalid
 * change never reaches the history (item 3: only valid commands are persisted).
 */
export function buildCommand(intent: CommandIntent): CommandBuildResult {
  switch (intent.kind) {
    case "morph": {
      const slider = sliderOf(intent.slider_id);
      if (!slider) {
        return fail("unknown_target", `slider '${intent.slider_id}' não existe no catálogo`, "slider_id");
      }
      const problem = finite(intent.value, `mrf_${slider.id}`) ?? inRange(intent.value, slider.min, slider.max, `mrf_${slider.id}`);
      if (problem) return problem;
      if (intent.current_value !== undefined && Math.abs(intent.current_value - intent.value) <= NO_OP_EPSILON) {
        return fail("no_op", `'${slider.id}' já está em ${intent.value}`, slider.id);
      }
      return {
        ok: true,
        command: { kind: "set_morph_value", target: morphIdForSlider(slider.id), value: intent.value },
      };
    }

    case "reset_morphs": {
      if (intent.override_count === 0) {
        return fail("no_op", "não há morphs para redefinir (todos nos valores canônicos)");
      }
      return { ok: true, command: { kind: "reset_morphs" } };
    }

    case "base_gender": {
      if (intent.current_gender && intent.current_gender === intent.gender) {
        return fail("no_op", `o personagem já é ${intent.gender}`, "gender");
      }
      return { ok: true, command: { kind: "set_base_gender", gender: intent.gender === "male" ? "Male" : "Female" } };
    }

    case "somatotype": {
      const components: Array<[string, number]> = [
        ["endomorph", intent.endo],
        ["mesomorph", intent.meso],
        ["ectomorph", intent.ecto],
      ];
      for (const [field, value] of components) {
        const problem = finite(value, field) ?? inRange(value, 0, 1, `somatotype.${field}`);
        if (problem) return problem;
      }
      if (intent.endo + intent.meso + intent.ecto <= NO_OP_EPSILON) {
        return fail("invalid_value", "os componentes do somatótipo não podem ser todos zero", "somatotype");
      }
      const current = intent.current;
      if (
        current &&
        Math.abs(current.endo - intent.endo) <= NO_OP_EPSILON &&
        Math.abs(current.meso - intent.meso) <= NO_OP_EPSILON &&
        Math.abs(current.ecto - intent.ecto) <= NO_OP_EPSILON
      ) {
        return fail("no_op", "o somatótipo já está nesses valores", "somatotype");
      }
      return {
        ok: true,
        command: { kind: "set_somatotype", endomorph: intent.endo, mesomorph: intent.meso, ectomorph: intent.ecto },
      };
    }

    case "gender_dimorphism": {
      const problem = finite(intent.value, "gender_dimorphism") ?? inRange(intent.value, 0, 1, "gender_dimorphism");
      if (problem) return problem;
      if (intent.current_value !== undefined && Math.abs(intent.current_value - intent.value) <= NO_OP_EPSILON) {
        return fail("no_op", `o dimorfismo já é ${intent.value}`, "gender_dimorphism");
      }
      return { ok: true, command: { kind: "set_gender_dimorphism", value: intent.value } };
    }

    case "proportions": {
      const patch: ProportionsIntent = {};
      for (const key of PROPORTION_KEYS) {
        const value = intent.patch[key];
        if (value === undefined) continue;
        const problem = finite(value, key);
        if (problem) return problem;
        patch[key] = value;
      }
      if (Object.keys(patch).length === 0) {
        return fail("no_op", "patch de proporções vazio");
      }
      return { ok: true, command: { kind: "set_proportions", ...patch } };
    }

    case "camera": {
      const command: CommandWire = { kind: "set_camera" };
      for (const [field, value] of [
        ["eye", intent.eye],
        ["target", intent.target],
        ["up", intent.up],
      ] as const) {
        const problem = buildVec3(value, field);
        if (problem) return problem;
        if (value) (command as Record<string, unknown>)[field] = [...value];
      }
      if (intent.fov_degrees !== undefined) {
        const problem = finite(intent.fov_degrees, "fov_degrees") ?? inRange(intent.fov_degrees, 1, 179, "fov_degrees");
        if (problem) return problem;
        (command as Record<string, unknown>)["fov_degrees"] = intent.fov_degrees;
      }
      if (Object.keys(command).length === 1) {
        return fail("no_op", "patch de câmera vazio");
      }
      return { ok: true, command };
    }

    case "orbit": {
      const problem = finite(intent.azimuth, "azimuth") ?? finite(intent.elevation, "elevation");
      if (problem) return problem;
      if (Math.abs(intent.azimuth) <= NO_OP_EPSILON && Math.abs(intent.elevation) <= NO_OP_EPSILON) {
        return fail("no_op", "órbita sem deslocamento");
      }
      return { ok: true, command: { kind: "orbit_camera", azimuth: intent.azimuth, elevation: intent.elevation } };
    }

    case "zoom": {
      const problem = finite(intent.factor, "factor");
      if (problem) return problem;
      if (intent.factor <= 0) {
        return fail("invalid_value", "'factor' precisa ser maior que 0", "factor");
      }
      if (Math.abs(intent.factor - 1) <= NO_OP_EPSILON) {
        return fail("no_op", "zoom unitário (factor 1) não altera a câmera", "factor");
      }
      if (intent.current_factor !== undefined && Math.abs(intent.current_factor - intent.factor) <= NO_OP_EPSILON) {
        return fail("no_op", `o zoom já é ${intent.factor}`, "factor");
      }
      return { ok: true, command: { kind: "zoom_camera", factor: intent.factor } };
    }

    case "pan": {
      const problem = finite(intent.dx, "dx") ?? finite(intent.dy, "dy");
      if (problem) return problem;
      if (Math.abs(intent.dx) <= NO_OP_EPSILON && Math.abs(intent.dy) <= NO_OP_EPSILON) {
        return fail("no_op", "pan sem deslocamento");
      }
      return { ok: true, command: { kind: "pan_camera", dx: intent.dx, dy: intent.dy } };
    }

    case "light": {
      if (intent.patch.light_id !== undefined && !isValidStableId(intent.patch.light_id, "light")) {
        return fail("unknown_target", `light_id '${intent.patch.light_id}' inválido`, "light_id");
      }
      const command: CommandWire = { kind: "set_light" };
      for (const key of BACKGROUND_KEYS) {
        const value = intent.patch[key];
        if (value === undefined) continue;
        const problem = Array.isArray(value)
          ? buildVec3(value, key)
          : finite(value, key);
        if (problem) return problem;
        (command as Record<string, unknown>)[key] = Array.isArray(value) ? [...value] : value;
      }
      if (Object.keys(command).length === 1) {
        return fail("no_op", "patch de iluminação vazio");
      }
      if (intent.patch.light_id !== undefined) {
        (command as Record<string, unknown>)["light_id"] = intent.patch.light_id;
      }
      return { ok: true, command };
    }

    case "material": {
      if (intent.material_id !== undefined && !isValidStableId(intent.material_id, "material")) {
        return fail("unknown_target", `material_id '${intent.material_id}' inválido`, "material_id");
      }
      const patch: MaterialPatchWire = {};
      let touched = 0;
      for (const [key, value] of Object.entries(intent.patch)) {
        if (value === undefined) continue;
        const problem = Array.isArray(value) ? buildVec3w(value, key) : finite(value, key);
        if (problem) return problem;
        (patch as Record<string, unknown>)[key] = Array.isArray(value) ? [...value] : value;
        touched++;
      }
      if (touched === 0) {
        return fail("no_op", "patch de material vazio");
      }
      return intent.material_id !== undefined
        ? { ok: true, command: { kind: "set_material_params", material_id: intent.material_id, patch } }
        : { ok: true, command: { kind: "set_material_params", patch } };
    }

    case "node_visibility": {
      if (!isValidStableId(intent.node_id, "node")) {
        return fail("unknown_target", `node_id '${intent.node_id}' inválido`, "node_id");
      }
      if (intent.current_visible !== undefined && intent.current_visible === intent.visible) {
        return fail("no_op", `o nó já está ${intent.visible ? "visível" : "oculto"}`, "visible");
      }
      return { ok: true, command: { kind: "set_node_visibility", node_id: intent.node_id, visible: intent.visible } };
    }

    case "node_mesh": {
      if (!isValidStableId(intent.node_id, "node")) {
        return fail("unknown_target", `node_id '${intent.node_id}' inválido`, "node_id");
      }
      if (intent.mesh_uri === undefined || intent.mesh_uri === null) {
        return { ok: true, command: { kind: "set_node_mesh", node_id: intent.node_id, mesh: null } };
      }
      if (intent.mesh_uri.length === 0) {
        return fail("invalid_value", "mesh_uri vazio", "mesh_uri");
      }
      const mesh = { asset_id: assetIdForUri(intent.mesh_uri) };
      const withPrimitive =
        intent.primitive_index !== undefined
          ? { ...mesh, primitive_index: intent.primitive_index }
          : mesh;
      return { ok: true, command: { kind: "set_node_mesh", node_id: intent.node_id, mesh: withPrimitive } };
    }

    case "preset": {
      if (!["mannequin", "cube", "sphere"].includes(intent.preset)) {
        return fail("invalid_value", `preset '${intent.preset}' desconhecido`, "preset");
      }
      return { ok: true, command: { kind: "load_mesh_preset", preset: intent.preset } };
    }

    case "background_color": {
      for (let index = 0; index < 4; index++) {
        const problem =
          finite(intent.color[index], `background_color[${index}]`) ??
          inRange(intent.color[index], 0, 1, `background_color[${index}]`);
        if (problem) return problem;
      }
      if (
        intent.current_color &&
        intent.current_color.every((value, index) => Math.abs(value - intent.color[index]) <= NO_OP_EPSILON)
      ) {
        return fail("no_op", "a cor de fundo já é essa", "background_color");
      }
      return {
        ok: true,
        command: { kind: "set_background_color", color: [...intent.color] as [number, number, number, number] },
      };
    }

    case "render_settings": {
      if (intent.msaa_samples === undefined && intent.tonemap === undefined) {
        return fail("no_op", "configurações de render vazias");
      }
      const command: CommandWire = { kind: "set_render_settings" };
      if (intent.msaa_samples !== undefined) {
        if (!MSAA_SAMPLES.includes(intent.msaa_samples as (typeof MSAA_SAMPLES)[number])) {
          return fail(
            "invalid_value",
            `msaa_samples deve ser um de ${MSAA_SAMPLES.join(", ")}, recebido ${intent.msaa_samples}`,
            "msaa_samples"
          );
        }
        (command as Record<string, unknown>)["msaa_samples"] = intent.msaa_samples;
      }
      if (intent.tonemap !== undefined) {
        if (!["none", "reinhard", "neutral"].includes(intent.tonemap)) {
          return fail("invalid_value", `tonemap '${intent.tonemap}' desconhecido`, "tonemap");
        }
        (command as Record<string, unknown>)["tonemap"] = intent.tonemap;
      }
      return { ok: true, command };
    }

    case "rename_project": {
      if (typeof intent.name !== "string" || intent.name.trim().length === 0) {
        return fail("invalid_value", "o nome do projeto não pode ser vazio", "name");
      }
      if (intent.current_name !== undefined && intent.current_name === intent.name) {
        return fail("no_op", `o projeto já se chama '${intent.name}'`, "name");
      }
      return { ok: true, command: { kind: "rename_project", name: intent.name } };
    }

    case "batch": {
      if (intent.intents.length === 0) {
        return fail("empty_batch", "batch sem comandos");
      }
      const commands: CommandWire[] = [];
      for (const child of intent.intents) {
        const built = buildCommand(child);
        if (!built.ok) {
          // A batch is atomic in the core: if one part is invalid, none of it is
          // sent (the failure names the offending intent).
          return { ok: false, code: built.code, field: built.field, reason: `batch: ${built.reason}` };
        }
        commands.push(built.command);
      }
      return { ok: true, command: { kind: "batch", commands } };
    }

    default: {
      const exhaustive: never = intent;
      return fail("invalid_value", `intent desconhecida: ${JSON.stringify(exhaustive)}`);
    }
  }
}

/** 4-component vector check for material colors (RGBA). */
function buildVec3w(value: unknown, field: string): CommandBuildFailure | null {
  if (!Array.isArray(value) || value.length !== 4) {
    return fail("invalid_value", `'${field}' precisa ser um vetor de 4 números`, field);
  }
  for (let index = 0; index < 4; index++) {
    const problem = finite(value[index], `${field}[${index}]`);
    if (problem) return problem;
  }
  return null;
}

/** Stable prefix of a morph id, exposed for telemetry/tests. */
export const MORPH_ID_PREFIX = ID_PREFIXES.morph;
