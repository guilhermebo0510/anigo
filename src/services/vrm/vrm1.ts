/**
 * ANIGO — parser das extensões VRM 1.0 (Fase 2, issue #25).
 *
 * Deserializa as quatro extensões oficiais da especificação VRM 1.0:
 *
 *  - `VRMC_vrm`            — metadados, humanoid (mapeamento de ossos) e
 *                            expressões faciais (preset + custom → índice de
 *                            morph target);
 *  - `VRMC_materials_mtoon`— parâmetros MToon por material (subEmission,
 *                            shadowColor, shadeShift, shadeToony, rim,
 *                            lightDirection, specular, smooth, sphere/matcap);
 *  - `VRMC_springBone`     — rig secundário: grupos de juntas (molas) e
 *                            colisores esféricos/capsulares;
 *  - `VRMC_node_constraint`— limites de rotação/traução/escala por nó.
 *
 * O parse é *tolerante a omissões opcionais* (cada campo segue o default da
 * spec) e *rigoroso no que existe* (estrutura inválida vira `Vrm1Error` com
 * código estável — nunca valor silenciosamente errado).
 */

import type { Gltf, GltfMaterial, ParsedGltfModel } from "./gltf2";

// ---------------------------------------------------------------------------
// Tipos VRM 1.0 (spec: https://github.com/vrm-c/vrm-specification)
// ---------------------------------------------------------------------------

export interface Vrm1Meta {
  title: string;
  author: string;
  version: string;
  year: number;
  license: { name?: string; vocabulary?: string };
  contactInformation: string;
  metadata: Array<{ type: string; value: string }>;
  reference: string;
}

export interface Vrm1HumanoidBone {
  /** Índice do nó glTF (null = mapeamento ausente). */
  node: number | null;
  name?: string;
}

export interface Vrm1Humanoid {
  humanBones: Record<string, Vrm1HumanoidBone>;
}

export type Vrm1ExpressionPresetName =
  | "neutral" | "angry" | "sad" | "happy" | "relaxed" | "aha" | "fun"
  | "sleepy" | "surprised" | "upset" | "ajishii" | "akubou" | "airy"
  | "frightened" | "disgusted" | "confused";

export const VRM1_EXPRESSION_PRESETS: readonly Vrm1ExpressionPresetName[] = [
  "neutral", "angry", "sad", "happy", "relaxed", "aha", "fun",
  "sleepy", "surprised", "upset", "ajishii", "akubou", "airy",
  "frightened", "disgusted", "confused",
];

export interface Vrm1ExpressionBinding {
  /** Índice do morph target na malha facial. */
  blendShape: number;
  isolated?: boolean;
}

export interface Vrm1Expression {
  preset: Record<Vrm1ExpressionPresetName, Vrm1ExpressionBinding>;
  custom: Record<string, Vrm1ExpressionBinding>;
}

export interface Vrm1SpringJoint {
  node: number;
  distance: number;
  hitRadius?: number;
  gravityPower?: number;
  gravityDir?: [number, number, number];
  springStiffness?: number;
  springDamping?: number;
}

export interface Vrm1SpringGroup {
  name: string;
  center: Vrm1SpringJoint;
  joints: Vrm1SpringJoint[];
  colliderGroups: number[];
}

export interface Vrm1SphereCollider {
  group: number;
  node: number;
  radius: number;
  offset?: [number, number, number];
}

export interface Vrm1CapsuleCollider {
  group: number;
  node: number;
  from: [number, number, number];
  to: [number, number, number];
  radius: number;
}

export interface Vrm1Colliders {
  spheres: Vrm1SphereCollider[];
  capsules: Vrm1CapsuleCollider[];
}

export interface Vrm1SpringBone {
  groups: Vrm1SpringGroup[];
  colliders: Vrm1Colliders[];
}

/** MToon (VRMC_materials_mtoon) — parâmetros por material. */
export interface Vrm1MToon {
  mainTex: number | null;
  subEmissionColor: [number, number, number, number];
  subEmissionTexture: number | null;
  multiply: [number, number, number, number];
  shadowColor: [number, number, number, number];
  shadeShift: number;
  shadeToony: number;
  lightColor: [number, number, number];
  rimColor: [number, number, number, number];
  rimPower: number;
  rimLight: boolean;
  rimLightColor: [number, number, number, number];
  lightDirection: [number, number, number];
  specularColor: [number, number, number, number];
  specularPower: number;
  useSmooth: boolean;
  smoothColor: [number, number, number, number];
  useSphere: boolean;
  sphereMode: "normal" | "additive";
  useMatCap: boolean;
}

export interface Vrm1NodeConstraintBound {
  min: number[];
  max: number[];
}

export interface Vrm1NodeConstraint {
  rotateOffset: number[] | null;
  rotateLimit: Vrm1NodeConstraintBound | null;
  translateOffset: number[] | null;
  translateLimit: Vrm1NodeConstraintBound | null;
  scaleOffset: number[] | null;
  scaleLimit: Vrm1NodeConstraintBound | null;
}

export interface ParsedVrm1 {
  meta: Vrm1Meta;
  humanoid: Vrm1Humanoid;
  expression: Vrm1Expression;
  /** `null` quando o modelo não tem `VRMC_springBone`. */
  springBone: Vrm1SpringBone | null;
  /** material index → parâmetros MToon (ausente = material PBR padrão). */
  materials: Map<number, Vrm1MToon>;
  /** node index → constraint. */
  nodeConstraints: Map<number, Vrm1NodeConstraint>;
  warnings: string[];
}

// ---------------------------------------------------------------------------
// Erros
// ---------------------------------------------------------------------------

export type Vrm1ErrorCode =
  | "NOT_VRM"
  | "BAD_VRM_EXTENSION"
  | "BAD_META"
  | "BAD_HUMANOID"
  | "BAD_EXPRESSION"
  | "BAD_MTOON"
  | "BAD_SPRINGBONE"
  | "BAD_NODE_CONSTRAINT"
  | "UNKNOWN_ERROR";

export class Vrm1Error extends Error {
  readonly code: Vrm1ErrorCode;
  constructor(code: Vrm1ErrorCode, message: string) {
    super(`[vrm1 ${code}] ${message}`);
    this.name = "Vrm1Error";
    this.code = code;
  }
}

function fail(code: Vrm1ErrorCode, message: string): never {
  throw new Vrm1Error(code, message);
}

// ---------------------------------------------------------------------------
// Helpers de leitura tolerante
// ---------------------------------------------------------------------------

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function asNumber(value: unknown, fallback: number, where: string): number {
  if (value === undefined || value === null) return fallback;
  if (typeof value !== "number" || !Number.isFinite(value)) {
    fail("BAD_MTOON", `${where}: número esperado, recebido ${String(value)}`);
  }
  return value;
}

function asBool(value: unknown, fallback: boolean, where: string): boolean {
  if (value === undefined || value === null) return fallback;
  if (typeof value !== "boolean") fail("BAD_MTOON", `${where}: boolean esperado`);
  return value;
}

function asVec(value: unknown, size: number, fallback: number[], where: string): number[] {
  if (value === undefined || value === null) return fallback.slice();
  if (!Array.isArray(value) || value.length !== size) {
    fail("BAD_MTOON", `${where}: vetor de ${size} esperado`);
  }
  for (const component of value) {
    if (typeof component !== "number" || !Number.isFinite(component)) {
      fail("BAD_MTOON", `${where}: componente não numérico`);
    }
  }
  return value.slice();
}

function asNode(value: unknown, where: string): number | null {
  if (value === null) return null;
  if (typeof value !== "number" || !Number.isInteger(value)) {
    fail("BAD_HUMANOID", `${where}: node deve ser inteiro ou null`);
  }
  return value;
}

// ---------------------------------------------------------------------------
// Parse
// ---------------------------------------------------------------------------

/** `true` quando o documento declara e exige `VRMC_vrm` (um .vrm de fato). */
export function isVrmDocument(gltf: Gltf): boolean {
  const required = gltf.asset?.extensionsRequired ?? [];
  const used = gltf.asset?.extensionsUsed ?? [];
  return required.includes("VRMC_vrm") || used.includes("VRMC_vrm");
}

function parseMeta(raw: unknown, warnings: string[]): Vrm1Meta {
  if (!isRecord(raw)) fail("BAD_META", "VRMC_vrm.meta ausente");
  const meta: Vrm1Meta = {
    title: typeof raw["title"] === "string" ? (raw["title"] as string) : "Untitled VRM",
    author: typeof raw["author"] === "string" ? (raw["author"] as string) : "",
    version: typeof raw["version"] === "string" ? (raw["version"] as string) : "1.0.0",
    year: typeof raw["year"] === "number" ? (raw["year"] as number) : new Date().getFullYear(),
    license: isRecord(raw["license"])
      ? {
          name: typeof raw["license"]["name"] === "string" ? (raw["license"] as Record<string, unknown>)["name"] as string : undefined,
          vocabulary: typeof raw["license"]["vocabulary"] === "string" ? (raw["license"] as Record<string, unknown>)["vocabulary"] as string : undefined,
        }
      : {},
    contactInformation: typeof raw["contactInformation"] === "string" ? (raw["contactInformation"] as string) : "",
    metadata: Array.isArray(raw["metadata"])
      ? (raw["metadata"] as unknown[]).flatMap((entry) =>
          isRecord(entry) && typeof entry["type"] === "string" && typeof entry["value"] === "string"
            ? [{ type: entry["type"] as string, value: entry["value"] as string }]
            : [],
        )
      : [],
    reference: typeof raw["reference"] === "string" ? (raw["reference"] as string) : "",
  };
  if (!raw["title"]) warnings.push("VRM meta.title ausente — usando 'Untitled VRM'");
  if (!raw["author"]) warnings.push("VRM meta.author ausente");
  return meta;
}

function parseHumanoid(raw: unknown): Vrm1Humanoid {
  if (!isRecord(raw)) fail("BAD_HUMANOID", "VRMC_vrm.humanoid ausente");
  const humanBonesRaw = raw["humanBones"];
  if (!isRecord(humanBonesRaw)) fail("BAD_HUMANOID", "VRMC_vrm.humanoid.humanBones ausente");
  const humanBones: Record<string, Vrm1HumanoidBone> = {};
  for (const [boneName, entry] of Object.entries(humanBonesRaw)) {
    if (!isRecord(entry)) fail("BAD_HUMANOID", `humanBones.${boneName} inválido`);
    humanBones[boneName] = {
      node: asNode(entry["node"], `humanBones.${boneName}.node`),
      name: typeof entry["name"] === "string" ? (entry["name"] as string) : undefined,
    };
  }
  return { humanBones };
}

function parseExpression(raw: unknown): Vrm1Expression {
  if (!isRecord(raw)) fail("BAD_EXPRESSION", "VRMC_vrm.expression ausente");
  const presetRaw = raw["preset"];
  if (!isRecord(presetRaw)) fail("BAD_EXPRESSION", "VRMC_vrm.expression.preset ausente");
  const preset: Record<Vrm1ExpressionPresetName, Vrm1ExpressionBinding> = {} as Vrm1Expression["preset"];
  for (const presetName of VRM1_EXPRESSION_PRESETS) {
    const entry = presetRaw[presetName];
    if (!isRecord(entry)) {
      fail("BAD_EXPRESSION", `expression.preset.${presetName} ausente (os 16 presets são obrigatórios)`);
    }
    const blendShape = asNumber(entry["blendShape"], -1, `expression.preset.${presetName}.blendShape`);
    if (blendShape < 0 || !Number.isInteger(blendShape)) {
      fail("BAD_EXPRESSION", `expression.preset.${presetName}.blendShape deve ser inteiro >= 0`);
    }
    preset[presetName] = {
      blendShape,
      isolated: asBool(entry["isolated"], false, `expression.preset.${presetName}.isolated`),
    };
  }
  const custom: Record<string, Vrm1ExpressionBinding> = {};
  const customRaw = raw["custom"];
  if (customRaw !== undefined) {
    if (!isRecord(customRaw)) fail("BAD_EXPRESSION", "expression.custom inválido");
    for (const [name, entry] of Object.entries(customRaw)) {
      if (!isRecord(entry)) fail("BAD_EXPRESSION", `expression.custom.${name} inválido`);
      const blendShape = asNumber(entry["blendShape"], -1, `expression.custom.${name}.blendShape`);
      if (blendShape < 0 || !Number.isInteger(blendShape)) {
        fail("BAD_EXPRESSION", `expression.custom.${name}.blendShape deve ser inteiro >= 0`);
      }
      custom[name] = {
        blendShape,
        isolated: asBool(entry["isolated"], false, `expression.custom.${name}.isolated`),
      };
    }
  }
  return { preset, custom };
}

function parseMToon(raw: unknown, where: string): Vrm1MToon {
  if (!isRecord(raw)) fail("BAD_MTOON", `${where}: bloco VRMC_materials_mtoon inválido`);
  const texIndex = (value: unknown, name: string): number | null => {
    if (value === null || value === undefined) return null;
    const record = isRecord(value) ? value : null;
    if (!record || typeof record["index"] !== "number") {
      fail("BAD_MTOON", `${where}.${name}: texture index inválido`);
    }
    return record["index"] as number;
  };
  const sphereMode = raw["sphereMode"];
  if (sphereMode !== undefined && sphereMode !== "normal" && sphereMode !== "additive") {
    fail("BAD_MTOON", `${where}.sphereMode: 'normal'|'additive' esperado`);
  }
  return {
    mainTex: texIndex(raw["mainTex"], "mainTex"),
    subEmissionColor: asVec(raw["subEmission"], 4, [1, 1, 1, 1], `${where}.subEmission`),
    subEmissionTexture: texIndex(raw["subEmissionTexture"], "subEmissionTexture"),
    multiply: asVec(raw["multiply"], 4, [1, 1, 1, 1], `${where}.multiply`),
    shadowColor: asVec(raw["shadowColor"], 4, [0.718, 0.831, 1, 1], `${where}.shadowColor`),
    shadeShift: asNumber(raw["shadeShift"], 0, `${where}.shadeShift`),
    shadeToony: asNumber(raw["shadeToony"], 1, `${where}.shadeToony`),
    lightColor: asVec(raw["lightColor"], 3, [1, 1, 1], `${where}.lightColor`),
    rimColor: asVec(raw["rimColor"], 4, [1, 1, 1, 0], `${where}.rimColor`),
    rimPower: asNumber(raw["rimPower"], 1, `${where}.rimPower`),
    rimLight: asBool(raw["rimLight"], false, `${where}.rimLight`),
    rimLightColor: asVec(raw["rimLightColor"], 4, [1, 1, 1, 1], `${where}.rimLightColor`),
    lightDirection: asVec(raw["lightDirection"], 3, [0, 0, 1], `${where}.lightDirection`),
    specularColor: asVec(raw["specularColor"], 4, [1, 1, 1, 1], `${where}.specularColor`),
    specularPower: asNumber(raw["specularPower"], 1, `${where}.specularPower`),
    useSmooth: asBool(raw["useSmooth"], false, `${where}.useSmooth`),
    smoothColor: asVec(raw["smoothColor"], 4, [0.7, 0.7, 0.7, 0.5], `${where}.smoothColor`),
    useSphere: asBool(raw["useSphere"], false, `${where}.useSphere`),
    sphereMode: (sphereMode as "normal" | "additive") ?? "normal",
    useMatCap: asBool(raw["useMatCap"], false, `${where}.useMatCap`),
  };
}

function parseSpringJoint(raw: unknown, where: string): Vrm1SpringJoint {
  if (!isRecord(raw)) fail("BAD_SPRINGBONE", `${where}: joint inválido`);
  const node = asNode(raw["node"], `${where}.node`);
  if (node === null) fail("BAD_SPRINGBONE", `${where}.node ausente`);
  const distance = asNumber(raw["distance"], 0, `${where}.distance`);
  if (distance < 0) fail("BAD_SPRINGBONE", `${where}.distance deve ser >= 0`);
  return {
    node,
    distance,
    hitRadius: asNumber(raw["hitRadius"], 0.05, `${where}.hitRadius`),
    gravityPower: asNumber(raw["gravityPower"], 0, `${where}.gravityPower`),
    gravityDir: asVec(raw["gravityDir"], 3, [0, -1, 0], `${where}.gravityDir`),
    springStiffness: asNumber(raw["springStiffness"], 0.5, `${where}.springStiffness`),
    springDamping: asNumber(raw["springDamping"], 0.5, `${where}.springDamping`),
  };
}

function parseSpringBone(raw: unknown, nodeCount: number, warnings: string[]): Vrm1SpringBone {
  if (!isRecord(raw)) fail("BAD_SPRINGBONE", "VRMC_springBone inválido");
  const rig = raw["secondaryRig"];
  if (!isRecord(rig)) fail("BAD_SPRINGBONE", "VRMC_springBone.secondaryRig ausente");
  const checkNode = (node: number, where: string): void => {
    if (node < 0 || node >= nodeCount) {
      warnings.push(`springBone ${where}.node=${node} fora da faixa de nodes — mola será ignorada em runtime`);
    }
  };
  const groupsRaw = Array.isArray(rig["groups"]) ? (rig["groups"] as unknown[]) : [];
  // Spec: `colliders` é um ARRAY de objetos {spheres?, capsules?}. Aceitamos
  // também o formato objeto único (algumas ferramentas antigas).
  const collidersValue = rig["colliders"];
  const collidersRaw = Array.isArray(collidersValue)
    ? (collidersValue as unknown[])
    : isRecord(collidersValue)
      ? [collidersValue]
      : [];
  const groups: Vrm1SpringGroup[] = groupsRaw.flatMap((groupRaw, i) => {
    if (!isRecord(groupRaw)) fail("BAD_SPRINGBONE", `groups[${i}] inválido`);
    const jointsRaw = Array.isArray(groupRaw["joints"]) ? (groupRaw["joints"] as unknown[]) : [];
    const center = parseSpringJoint(groupRaw["center"], `groups[${i}].center`);
    const joints = jointsRaw.map((jointRaw, j) => parseSpringJoint(jointRaw, `groups[${i}].joints[${j}]`));
    checkNode(center.node, `groups[${i}].center`);
    joints.forEach((joint, j) => checkNode(joint.node, `groups[${i}].joints[${j}]`));
    return [{
      name: typeof groupRaw["name"] === "string" ? (groupRaw["name"] as string) : `group_${i}`,
      center,
      joints,
      colliderGroups: Array.isArray(groupRaw["colliderGroups"])
        ? (groupRaw["colliderGroups"] as unknown[]).filter(
            (value): value is number => typeof value === "number" && Number.isInteger(value),
          )
        : [],
    }];
  });
  const colliders: Vrm1Colliders = {
    spheres: (Array.isArray(collidersRaw) ? (collidersRaw as unknown[]) : []).flatMap((colliderRaw, i) => {
      if (!isRecord(colliderRaw)) fail("BAD_SPRINGBONE", `colliders[${i}] inválido`);
      const spheres = Array.isArray(colliderRaw["spheres"]) ? (colliderRaw["spheres"] as unknown[]) : [];
      return spheres.map((sphereRaw, j) => {
        if (!isRecord(sphereRaw)) fail("BAD_SPRINGBONE", `colliders[${i}].spheres[${j}] inválido`);
        const node = asNode(sphereRaw["node"], `colliders[${i}].spheres[${j}].node`) ?? 0;
        checkNode(node, `colliders[${i}].spheres[${j}]`);
        return {
          group: asNumber(sphereRaw["group"], 0, `colliders[${i}].spheres[${j}].group`),
          node,
          radius: asNumber(sphereRaw["radius"], 0, `colliders[${i}].spheres[${j}].radius`),
          offset: asVec(sphereRaw["offset"], 3, [0, 0, 0], `colliders[${i}].spheres[${j}].offset`),
        } as Vrm1SphereCollider;
      });
    }),
    capsules: (Array.isArray(collidersRaw) ? (collidersRaw as unknown[]) : []).flatMap((colliderRaw, i) => {
      if (!isRecord(colliderRaw)) fail("BAD_SPRINGBONE", `colliders[${i}] inválido`);
      const capsules = Array.isArray(colliderRaw["capsules"]) ? (colliderRaw["capsules"] as unknown[]) : [];
      return capsules.map((capsuleRaw, j) => {
        if (!isRecord(capsuleRaw)) fail("BAD_SPRINGBONE", `colliders[${i}].capsules[${j}] inválido`);
        const node = asNode(capsuleRaw["node"], `colliders[${i}].capsules[${j}].node`) ?? 0;
        checkNode(node, `colliders[${i}].capsules[${j}]`);
        return {
          group: asNumber(capsuleRaw["group"], 0, `colliders[${i}].capsules[${j}].group`),
          node,
          from: asVec(capsuleRaw["from"], 3, [0, 0, 0], `colliders[${i}].capsules[${j}].from`),
          to: asVec(capsuleRaw["to"], 3, [0, 0, 0], `colliders[${i}].capsules[${j}].to`),
          radius: asNumber(capsuleRaw["radius"], 0, `colliders[${i}].capsules[${j}].radius`),
        } as Vrm1CapsuleCollider;
      });
    }),
  };
  return { groups, colliders };
}

function parseNodeConstraint(raw: unknown, nodeIndex: number): Vrm1NodeConstraint {
  if (!isRecord(raw)) fail("BAD_NODE_CONSTRAINT", `nodes[${nodeIndex}]: constraint inválido`);
  const bound = (value: unknown, where: string): Vrm1NodeConstraintBound | null => {
    if (value === null || value === undefined) return null;
    if (!isRecord(value)) fail("BAD_NODE_CONSTRAINT", `${where}: bound inválido`);
    const min = asVec(value["min"], 3, [0, 0, 0], `${where}.min`);
    const max = asVec(value["max"], 3, [0, 0, 0], `${where}.max`);
    return { min, max };
  };
  return {
    rotateOffset: raw["rotateOffset"] === null || raw["rotateOffset"] === undefined
      ? null
      : asVec(raw["rotateOffset"], 3, [0, 0, 0], "rotateOffset"),
    rotateLimit: bound(raw["rotateLimit"], "rotateLimit"),
    translateOffset: raw["translateOffset"] === null || raw["translateOffset"] === undefined
      ? null
      : asVec(raw["translateOffset"], 3, [0, 0, 0], "translateOffset"),
    translateLimit: bound(raw["translateLimit"], "translateLimit"),
    scaleOffset: raw["scaleOffset"] === null || raw["scaleOffset"] === undefined
      ? null
      : asVec(raw["scaleOffset"], 3, [0, 0, 0], "scaleOffset"),
    scaleLimit: bound(raw["scaleLimit"], "scaleLimit"),
  };
}

// ---------------------------------------------------------------------------
// API principal
// ---------------------------------------------------------------------------

/**
 * Extrai a camada VRM 1.0 de um documento glTF já parseado.
 * Lança `Vrm1Error` quando o documento não é um VRM 1.0 ou está malformado.
 */
export function parseVrm1(model: ParsedGltfModel): ParsedVrm1 {
  const { gltf } = model;
  if (!isVrmDocument(gltf)) {
    fail("NOT_VRM", "documento não declara a extensão VRMC_vrm (não é um .vrm)");
  }
  const warnings: string[] = [...model.warnings];
  const vrmRaw = gltf.extensions?.["VRMC_vrm"];
  if (!isRecord(vrmRaw)) fail("BAD_VRM_EXTENSION", "extensões glTF sem bloco VRMC_vrm");

  const meta = parseMeta(vrmRaw["meta"], warnings);
  const humanoid = parseHumanoid(vrmRaw["humanoid"]);
  const expression = parseExpression(vrmRaw["expression"]);
  const springBone = vrmRaw["secondary"] !== undefined || gltf.extensions?.["VRMC_springBone"] !== undefined
    ? parseSpringBone(vrmRaw["secondary"] ?? gltf.extensions?.["VRMC_springBone"], (gltf.nodes ?? []).length, warnings)
    : null;

  const materials = new Map<number, Vrm1MToon>();
  for (let i = 0; i < (gltf.materials ?? []).length; i++) {
    const material = gltf.materials![i];
    const mtoonRaw = (material as GltfMaterial).extensions?.["VRMC_materials_mtoon"];
    if (mtoonRaw !== undefined) {
      materials.set(i, parseMToon(mtoonRaw, `materials[${i}]`));
    }
  }

  const nodeConstraints = new Map<number, Vrm1NodeConstraint>();
  for (let i = 0; i < (gltf.nodes ?? []).length; i++) {
    const constraintRaw = gltf.nodes![i].extensions?.["VRMC_node_constraint"];
    if (constraintRaw !== undefined) {
      nodeConstraints.set(i, parseNodeConstraint(constraintRaw, i));
    }
  }

  return { meta, humanoid, expression, springBone, materials, nodeConstraints, warnings };
}

// ---------------------------------------------------------------------------
// Mapeamento MToon → parâmetros de material anime do ANIGO
// ---------------------------------------------------------------------------

/**
 * Converte um material PBR+MToon de VRM para o dicionário de parâmetros do
 * material estilizado do ANIGO (mesmas chaves do `StylizedMaterial` do core).
 * A correspondência segue a convenção VRoid:
 *
 *  - `base_color`   ← `baseColorFactor × multiply`
 *  - `shade_color`  ← `baseColorFactor × shadowColor`
 *  - `shadow_threshold` ← 0.5 + `shadeShift` × 0.5 (deslocamento do limiar)
 *  - `toon_steps`   ← 2.0 quando `shadeToony` < 0.5 (duas bandas), senão 1.0
 *  - `rim_*`        ← `rimColor`/`rimPower`/`rimLight`
 *  - `spec_*`       ← `specularColor`/`specularPower`
 *  - emissão        ← `subEmission` (cor × 1.0, HDR preservado via intensidade)
 */
export interface MToonMaterialMapping {
  baseColor: [number, number, number, number];
  shadeColor: [number, number, number, number];
  shadowThreshold: number;
  toonSteps: number;
  rimColor: [number, number, number, number];
  rimIntensity: number;
  rimSpread: number;
  specularColor: [number, number, number, number];
  specIntensity: number;
  specPower: number;
  emissionColor: [number, number, number, number];
  emissionIntensity: number;
  /** Índice de textura de albedo (ou null). */
  mainTexture: number | null;
  useSphere: boolean;
  sphereMode: "normal" | "additive";
  useMatCap: boolean;
}

export function mapMToonToAnimeMaterial(material: GltfMaterial, mtoon: Vrm1MToon | undefined): MToonMaterialMapping {
  const pbr = material.pbrMetallicRoughness;
  const baseFactor: [number, number, number, number] =
    pbr && Array.isArray(pbr.baseColorFactor) && pbr.baseColorFactor.length === 4
      ? [pbr.baseColorFactor[0], pbr.baseColorFactor[1], pbr.baseColorFactor[2], pbr.baseColorFactor[3]]
      : [1, 1, 1, 1];
  const mul = mtoon ? mtoon.multiply : [1, 1, 1, 1];
  const baseColor: [number, number, number, number] = [
    baseFactor[0] * mul[0],
    baseFactor[1] * mul[1],
    baseFactor[2] * mul[2],
    baseFactor[3] * mul[3],
  ];
  const shadow = mtoon ? mtoon.shadowColor : [1, 1, 1, 1];
  const shadeColor: [number, number, number, number] = [
    baseColor[0] * shadow[0],
    baseColor[1] * shadow[1],
    baseColor[2] * shadow[2],
    baseColor[3] * shadow[3],
  ];
  return {
    baseColor,
    shadeColor,
    shadowThreshold: mtoon ? clamp01(0.5 + mtoon.shadeShift * 0.5) : 0.5,
    toonSteps: mtoon && mtoon.shadeToony < 0.5 ? 2.0 : 1.0,
    rimColor: mtoon ? mtoon.rimColor : [0.576, 0.773, 0.992, 1],
    rimIntensity: mtoon ? mtoon.rimPower * (mtoon.rimLight ? 1.0 : 0.8) : 0.8,
    rimSpread: mtoon ? clamp01(1.0 / Math.max(mtoon.rimPower, 0.1)) : 0.4,
    specularColor: mtoon ? mtoon.specularColor : [1, 1, 1, 1],
    specIntensity: mtoon ? clamp01(mtoon.specularPower * 0.5) : 0.4,
    specPower: mtoon ? Math.max(2, mtoon.specularPower * 24) : 32,
    emissionColor: mtoon ? mtoon.subEmissionColor : [1, 1, 1, 0],
    emissionIntensity: mtoon ? 1.0 : 0.0,
    mainTexture: mtoon ? mtoon.mainTex : (pbr?.baseColorTexture ? pbr.baseColorTexture.index : null),
    useSphere: mtoon ? mtoon.useSphere : false,
    sphereMode: mtoon ? mtoon.sphereMode : "normal",
    useMatCap: mtoon ? mtoon.useMatCap : false,
  };
}

function clamp01(value: number): number {
  return Math.min(1, Math.max(0, value));
}
