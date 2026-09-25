/**
 * ANIGO — Command v1 contract (TypeScript side).
 *
 * Mirrors `crates/anigo-core/src/command.rs`. Every persistent change the UI
 * performs must be expressed as one of these commands and sent to the core;
 * the UI never writes project data directly (ARQUITETURA_CANONICA_ANIGO §2.5).
 *
 * Wire format: internally tagged by `kind` (snake_case), field names identical
 * to Rust. A drift test (`tests/contracts/command_contract.test.ts`) parses the
 * Rust enum and asserts this file stays in sync.
 */

export const COMMAND_CONTRACT_VERSION = 1;

export type ChangeScope =
  | "base_geometry"
  | "deformation"
  | "shading"
  | "camera"
  | "presentation"
  | "project";

export type MeshPresetWire = "mannequin" | "cube" | "sphere";

/**
 * Specialized node type (mirrors Rust `hierarchy::NodeKind`).
 *
 * The type travels on the wire because the viewport needs it: clothing/hair
 * follow the armature (secondary motion), while `group`/`light`/`camera` nodes
 * carry no geometry of their own.
 */
export type NodeKindWire =
  | "character_root"
  | "humanoid_bone"
  | "clothing"
  | "hair"
  | "accessory"
  | "mesh"
  | "light"
  | "camera"
  | "group";

/**
 * Node types in canonical order — the same list (and the same order) as Rust
 * `hierarchy::NodeKind::ALL`, drift-checked by the command contract test.
 */
export const NODE_KINDS = [
  "character_root",
  "humanoid_bone",
  "clothing",
  "hair",
  "accessory",
  "mesh",
  "light",
  "camera",
  "group",
] as const satisfies readonly NodeKindWire[];

/** Volume ortográfico (issue #13) — espelha Rust `math::OrthographicBounds`. */
export interface OrthographicBoundsWire {
  left: number;
  right: number;
  bottom: number;
  top: number;
}

/**
 * Patch do modo de projeção da câmera (issue #13).
 *
 * `orthographic` + `ortho_height` é o que a UI manda (volume simétrico que
 * preserva o enquadramento); `ortho_bounds` existe para o inverso restaurar o
 * volume exato de um undo.
 */
export interface CameraProjectionPatchWire {
  orthographic?: boolean;
  ortho_height?: number;
  ortho_bounds?: OrthographicBoundsWire;
}

/** Local transform of a node (mirrors Rust `math::Transform`). */
export interface TransformWire {
  translation: [number, number, number];
  rotation: [number, number, number, number];
  scale: [number, number, number];
}

export type TonemapOperatorWire = "none" | "reinhard" | "neutral";

export interface MeshRefWire {
  asset_id: string;
  primitive_index?: number;
}

/** Patch of the stylized material — only `Some` fields are written (Rust `Option`). */
export interface MaterialPatchWire {
  base_color?: [number, number, number, number];
  shade_color?: [number, number, number, number];
  outline_color?: [number, number, number, number];
  outline_width?: number;
  shadow_threshold?: number;
  shadow_smoothness?: number;
  spec_intensity?: number;
  spec_power?: number;
  rim_intensity?: number;
  rim_spread?: number;
  hue_shift?: number;
  toon_steps?: number;
  specular_color?: [number, number, number, number];
  specular_softness?: number;
  specular_offset?: number;
  rim_color?: [number, number, number, number];
  outline_opacity?: number;
  outline_smoothness?: number;
  outline_depth_bias?: number;
  specular_size?: number;
  ao_intensity?: number;
}

export type CommandWire =
  | { kind: "set_morph_value"; target: string; value: number }
  | { kind: "reset_morphs" }
  | { kind: "set_base_gender"; gender: "Male" | "Female" }
  | { kind: "set_somatotype"; endomorph: number; mesomorph: number; ectomorph: number }
  | { kind: "set_gender_dimorphism"; value: number }
  | {
      kind: "set_proportions";
      head_scale?: number;
      head_ratio?: number;
      shoulder_width?: number;
      leg_length?: number;
      arm_length?: number;
      neck_length?: number;
      torso_length?: number;
      height_overall?: number;
    }
  | {
      kind: "set_camera";
      eye?: [number, number, number];
      target?: [number, number, number];
      up?: [number, number, number];
      fov_degrees?: number;
      projection?: CameraProjectionPatchWire;
    }
  | { kind: "orbit_camera"; azimuth: number; elevation: number }
  | { kind: "zoom_camera"; factor: number }
  | { kind: "pan_camera"; dx: number; dy: number }
  | {
      kind: "set_light";
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
  | { kind: "set_material_params"; material_id?: string; patch: MaterialPatchWire }
  | { kind: "set_node_visibility"; node_id: string; visible: boolean }
  | { kind: "set_node_mesh"; node_id: string; mesh?: MeshRefWire | null }
  | {
      kind: "add_node";
      node_id: string;
      name: string;
      parent_id?: string | null;
      node_kind?: NodeKindWire;
      transform?: TransformWire;
      mesh?: MeshRefWire | null;
      material_id?: string | null;
      index: number;
    }
  | { kind: "remove_node"; node_id: string }
  | { kind: "set_node_parent"; node_id: string; parent_id?: string | null }
  | {
      kind: "set_node_transform";
      node_id: string;
      translation?: [number, number, number];
      rotation?: [number, number, number, number];
      scale?: [number, number, number];
    }
  | { kind: "load_mesh_preset"; preset: MeshPresetWire }
  | { kind: "set_background_color"; color: [number, number, number, number] }
  // Issue #14: `graph_order`/`graph_disabled` (nomes do contrato) e o
  // `depth_prepass` viajam no `set_render_settings` (o teste de drift não
  // tolera comentários entre os campos do membro).
  | {
      kind: "set_render_settings";
      msaa_samples?: number;
      tonemap?: TonemapOperatorWire;
      graph_order?: string[];
      graph_disabled?: string[];
      depth_prepass?: boolean;
    }
  | { kind: "rename_project"; name: string }
  | { kind: "batch"; commands: CommandWire[] };

/** Kinds known by this contract version (drift-checked against the Rust enum). */
export const COMMAND_KINDS = [
  "set_morph_value",
  "reset_morphs",
  "set_base_gender",
  "set_somatotype",
  "set_gender_dimorphism",
  "set_proportions",
  "set_camera",
  "orbit_camera",
  "zoom_camera",
  "pan_camera",
  "set_light",
  "set_material_params",
  "set_node_visibility",
  "set_node_mesh",
  "add_node",
  "remove_node",
  "set_node_parent",
  "set_node_transform",
  "load_mesh_preset",
  "set_background_color",
  "set_render_settings",
  "rename_project",
  "batch",
] as const;

export type CommandKind = (typeof COMMAND_KINDS)[number];

/** Outcome returned by the core after a command (mirrors Rust `CommandOutcome`). */
export interface CommandOutcomeWire {
  sequence: number;
  revision: number;
  base_geometry_revision: number;
  description: string;
  scope: ChangeScope;
  affected: string[];
  can_undo: boolean;
  can_redo: boolean;
  undo_depth: number;
  redo_depth: number;
}

/** Minimal structural validation before sending a command to the core. */
export function isCommandWire(value: unknown): value is CommandWire {
  if (typeof value !== "object" || value === null) return false;
  const kind = (value as { kind?: unknown }).kind;
  return typeof kind === "string" && (COMMAND_KINDS as readonly string[]).includes(kind);
}
