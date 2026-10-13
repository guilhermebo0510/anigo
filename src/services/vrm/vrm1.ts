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
 *  - `VRMC_springBone`     — rig secundário no schema oficial 1.0 (`springs[].joints[]`,
 *                            `colliders[].shape`, `colliderGroups`, `center`), mais
 *                            `VRMC_springBone_extended_collider` (inside/plane); o formato
 *                            legado do ANIGO (`secondaryRig`) ainda é lido e convertido;
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

/** Versão da spec `VRMC_springBone` / `VRMC_springBone_extended_collider` emitida pelo exportador. */
export const SPRING_BONE_SPEC_VERSION = "1.0";

/**
 * Junta de uma cadeia (`springs[i].joints[j]`, schema oficial `VRMC_springBone-1.0`).
 * Defaults da spec: hitRadius=0, stiffness=1, gravityPower=0, gravityDir=[0,-1,0], dragForce=0.5.
 * A última junta da cadeia só serve de "cauda" (parâmetros não usados).
 */
export interface Vrm1SpringJoint {
  node: number;
  hitRadius: number;
  stiffness: number;
  gravityPower: number;
  gravityDir: [number, number, number];
  dragForce: number;
}

export interface Vrm1Spring {
  name?: string;
  joints: Vrm1SpringJoint[];
  /** Índices em `VRMC_springBone.colliderGroups`. */
  colliderGroups: number[];
  /** Nó raiz do "center space" (undefined = espaço do mundo). */
  center?: number;
}

/**
 * Forma de um collider. `sphere`/`capsule` com `inside=false` são a spec base;
 * `inside=true` e `plane` vêm de `VRMC_springBone_extended_collider`.
 */
export type Vrm1ColliderShape =
  | { sphere: { offset: [number, number, number]; radius: number; inside: boolean } }
  | { capsule: { offset: [number, number, number]; radius: number; tail: [number, number, number]; inside: boolean } }
  | { plane: { offset: [number, number, number]; normal: [number, number, number] } };

export interface Vrm1Collider {
  node: number;
  shape: Vrm1ColliderShape;
}

export interface Vrm1ColliderGroup {
  name?: string;
  /** Índices em `VRMC_springBone.colliders`. */
  colliders: number[];
}

export interface Vrm1SpringBone {
  specVersion: string;
  colliders: Vrm1Collider[];
  colliderGroups: Vrm1ColliderGroup[];
  springs: Vrm1Spring[];
}

/** `true` quando a forma precisa de `VRMC_springBone_extended_collider`. */
export function isExtendedColliderShape(shape: Vrm1ColliderShape): boolean {
  if ("plane" in shape) return true;
  if ("sphere" in shape) return shape.sphere.inside;
  return shape.capsule.inside;
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
  const required = gltf.extensionsRequired ?? gltf.asset?.extensionsRequired ?? [];
  const used = gltf.extensionsUsed ?? gltf.asset?.extensionsUsed ?? [];
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

// ---------------------------------------------------------------------------
// VRMC_springBone (schema oficial 1.0 + leitura tolerante do formato legado)
// ---------------------------------------------------------------------------

function sbFail(message: string): never {
  return fail("BAD_SPRINGBONE", message);
}

function sbNumber(value: unknown, fallback: number, where: string): number {
  if (value === undefined || value === null) return fallback;
  if (typeof value !== "number" || !Number.isFinite(value)) sbFail(`${where}: número finito esperado`);
  return value;
}

function sbVec3(value: unknown, fallback: [number, number, number], where: string): [number, number, number] {
  if (value === undefined || value === null) return [...fallback];
  if (!Array.isArray(value) || value.length !== 3) sbFail(`${where}: vetor de 3 números esperado`);
  return [sbNumber(value[0], 0, where), sbNumber(value[1], 0, where), sbNumber(value[2], 0, where)];
}

function sbNode(value: unknown, where: string): number {
  if (typeof value !== "number" || !Number.isInteger(value) || value < 0) {
    sbFail(`${where}: índice de nó (inteiro >= 0) obrigatório`);
  }
  return value;
}

function sbIndexList(value: unknown, where: string): number[] {
  if (value === undefined || value === null) return [];
  if (!Array.isArray(value)) sbFail(`${where}: lista de índices esperada`);
  return value.map((entry) => {
    if (typeof entry !== "number" || !Number.isInteger(entry) || entry < 0) {
      sbFail(`${where}: índice inteiro >= 0 esperado`);
    }
    return entry;
  });
}

function sbName(value: unknown): string | undefined {
  return typeof value === "string" ? value : undefined;
}

function nodeParents(nodes: ReadonlyArray<{ children?: number[] }>): Array<number | null> {
  const parents: Array<number | null> = nodes.map(() => null);
  nodes.forEach((node, i) => {
    for (const child of node.children ?? []) {
      if (Number.isInteger(child) && child >= 0 && child < parents.length) parents[child] = i;
    }
  });
  return parents;
}

function isAncestor(parents: Array<number | null>, ancestor: number, node: number): boolean {
  let cursor = parents[node] ?? null;
  let guard = 0;
  while (cursor !== null) {
    if (cursor === ancestor) return true;
    if (++guard > parents.length) return false;
    cursor = parents[cursor] ?? null;
  }
  return false;
}

function parseOfficialShape(shape: unknown, where: string): Vrm1ColliderShape {
  if (!isRecord(shape)) sbFail(`${where}: objeto esperado`);
  const nonNegative = (v: number, what: string): number => {
    if (v < 0) sbFail(`${where}.${what} deve ser >= 0`);
    return v;
  };
  const sphere = shape["sphere"];
  if (isRecord(sphere)) {
    return {
      sphere: {
        offset: sbVec3(sphere["offset"], [0, 0, 0], `${where}.sphere.offset`),
        radius: nonNegative(sbNumber(sphere["radius"], 0, `${where}.sphere.radius`), "sphere.radius"),
        inside: sphere["inside"] === true,
      },
    };
  }
  const capsule = shape["capsule"];
  if (isRecord(capsule)) {
    return {
      capsule: {
        offset: sbVec3(capsule["offset"], [0, 0, 0], `${where}.capsule.offset`),
        radius: nonNegative(sbNumber(capsule["radius"], 0, `${where}.capsule.radius`), "capsule.radius"),
        tail: sbVec3(capsule["tail"], [0, 0, 0], `${where}.capsule.tail`),
        inside: capsule["inside"] === true,
      },
    };
  }
  const plane = shape["plane"];
  if (isRecord(plane)) {
    const normal = sbVec3(plane["normal"], [0, 0, 1], `${where}.plane.normal`);
    const len = Math.hypot(normal[0], normal[1], normal[2]);
    if (len < 1e-6) sbFail(`${where}.plane.normal não pode ser o vetor nulo`);
    return {
      plane: {
        offset: sbVec3(plane["offset"], [0, 0, 0], `${where}.plane.offset`),
        normal: [normal[0] / len, normal[1] / len, normal[2] / len],
      },
    };
  }
  return sbFail(`${where}: shape deve ter exatamente um de sphere/capsule/plane`);
}

function parseOfficialJoint(raw: unknown, where: string): Vrm1SpringJoint {
  if (!isRecord(raw)) sbFail(`${where}: objeto esperado`);
  const joint: Vrm1SpringJoint = {
    node: sbNode(raw["node"], `${where}.node`),
    hitRadius: sbNumber(raw["hitRadius"], 0, `${where}.hitRadius`),
    stiffness: sbNumber(raw["stiffness"], 1, `${where}.stiffness`),
    gravityPower: sbNumber(raw["gravityPower"], 0, `${where}.gravityPower`),
    gravityDir: sbVec3(raw["gravityDir"], [0, -1, 0], `${where}.gravityDir`),
    dragForce: sbNumber(raw["dragForce"], 0.5, `${where}.dragForce`),
  };
  if (joint.hitRadius < 0) sbFail(`${where}.hitRadius deve ser >= 0`);
  if (joint.stiffness < 0) sbFail(`${where}.stiffness deve ser >= 0`);
  if (joint.dragForce < 0 || joint.dragForce > 1) sbFail(`${where}.dragForce deve estar em [0, 1]`);
  return joint;
}

function parseOfficialSpringBone(
  raw: Record<string, unknown>,
  nodes: ReadonlyArray<{ children?: number[] }>,
  warnings: string[],
): Vrm1SpringBone {
  if (typeof raw["specVersion"] !== "string") {
    warnings.push('VRMC_springBone.specVersion ausente (obrigatório na spec) — assumindo "1.0"');
  } else if (raw["specVersion"] !== SPRING_BONE_SPEC_VERSION) {
    warnings.push(`VRMC_springBone.specVersion="${raw["specVersion"]}" não é "1.0" — lendo como 1.0`);
  }
  const nodeCount = nodes.length;
  const parents = nodeParents(nodes);
  const checkNode = (node: number, where: string): void => {
    if (node >= nodeCount) warnings.push(`springBone ${where}.node=${node} fora da faixa de nodes — será ignorado em runtime`);
  };

  const colliders: Vrm1Collider[] = [];
  if (Array.isArray(raw["colliders"])) {
    raw["colliders"].forEach((entry: unknown, i: number) => {
      const w = `colliders[${i}]`;
      if (!isRecord(entry)) sbFail(`${w}: objeto esperado`);
      const node = sbNode(entry["node"], `${w}.node`);
      checkNode(node, w);
      const ext = isRecord(entry["extensions"]) ? entry["extensions"]["VRMC_springBone_extended_collider"] : undefined;
      const extShape = isRecord(ext) && isRecord(ext["shape"]) ? ext["shape"] : undefined;
      const baseShape = isRecord(entry["shape"]) ? entry["shape"] : undefined;
      if (extShape) {
        colliders.push({ node, shape: parseOfficialShape(extShape, `${w}.extensions.VRMC_springBone_extended_collider.shape`) });
      } else if (baseShape) {
        colliders.push({ node, shape: parseOfficialShape(baseShape, `${w}.shape`) });
      } else {
        sbFail(`${w}.shape obrigatório`);
      }
    });
  }

  const colliderGroups: Vrm1ColliderGroup[] = [];
  if (Array.isArray(raw["colliderGroups"])) {
    raw["colliderGroups"].forEach((entry: unknown, i: number) => {
      const w = `colliderGroups[${i}]`;
      if (!isRecord(entry)) sbFail(`${w}: objeto esperado`);
      const all = sbIndexList(entry["colliders"], `${w}.colliders`);
      const kept = all.filter((c) => c < colliders.length);
      if (kept.length !== all.length) warnings.push(`springBone ${w}.colliders referencia collider inexistente — referência descartada`);
      colliderGroups.push({ name: sbName(entry["name"]), colliders: kept });
    });
  }

  const springs: Vrm1Spring[] = [];
  const seen = new Map<number, number>();
  if (Array.isArray(raw["springs"])) {
    raw["springs"].forEach((entry: unknown, i: number) => {
      const w = `springs[${i}]`;
      if (!isRecord(entry)) sbFail(`${w}: objeto esperado`);
      if (!Array.isArray(entry["joints"])) sbFail(`${w}.joints obrigatório`);
      const joints = entry["joints"].map((jointRaw: unknown, j: number) => {
        const jw = `${w}.joints[${j}]`;
        const joint = parseOfficialJoint(jointRaw, jw);
        checkNode(joint.node, jw);
        const prev = seen.get(joint.node);
        if (prev !== undefined) {
          warnings.push(`springBone ${jw}.node=${joint.node} já pertence à cadeia springs[${prev}] (cadeias duplicadas são proibidas pela spec)`);
        }
        seen.set(joint.node, i);
        return joint;
      });
      if (joints.length < 2) warnings.push(`springBone ${w}.joints tem ${joints.length} junta(s); a spec exige >= 2 — cadeia sem efeito`);
      for (let j = 0; j + 1 < joints.length; j++) {
        const a = joints[j].node;
        const b = joints[j + 1].node;
        if (a < nodeCount && b < nodeCount && !isAncestor(parents, a, b)) {
          warnings.push(`springBone ${w}: nó ${b} não é descendente do nó ${a} — a spec exige joints[n] ancestral de joints[n+1]`);
        }
      }
      const groupsAll = sbIndexList(entry["colliderGroups"], `${w}.colliderGroups`);
      const groups = groupsAll.filter((g) => g < colliderGroups.length);
      if (groups.length !== groupsAll.length) warnings.push(`springBone ${w}.colliderGroups referencia grupo inexistente — referência descartada`);
      let center: number | undefined;
      if (entry["center"] !== undefined && entry["center"] !== null) {
        center = sbNode(entry["center"], `${w}.center`);
        checkNode(center, `${w}.center`);
      }
      springs.push({ name: sbName(entry["name"]), joints, colliderGroups: groups, ...(center !== undefined ? { center } : {}) });
    });
  }
  return { specVersion: SPRING_BONE_SPEC_VERSION, colliders, colliderGroups, springs };
}

/**
 * Formato legado do ANIGO (`secondaryRig.groups[]/colliders[]{spheres,capsules}`), anterior ao
 * schema oficial: convertido para o modelo novo para não perder arquivos já gerados.
 */
function parseLegacySpringBone(rig: Record<string, unknown>, nodeCount: number, warnings: string[]): Vrm1SpringBone {
  warnings.push(
    "VRMC_springBone em formato legado (secondaryRig) convertido para o schema oficial 1.0; reexporte para gravar o formato novo",
  );
  const checkNode = (node: number, where: string): void => {
    if (node >= nodeCount) warnings.push(`springBone ${where}.node=${node} fora da faixa — será ignorado em runtime`);
  };
  const legacyJoint = (raw: unknown, where: string): Vrm1SpringJoint => {
    if (!isRecord(raw)) return sbFail(`${where}: joint inválido`);
    const distance = sbNumber(raw["distance"], 0, `${where}.distance`);
    if (distance < 0) sbFail(`${where}.distance deve ser >= 0`);
    return {
      node: sbNode(raw["node"], `${where}.node`),
      hitRadius: Math.max(0, sbNumber(raw["hitRadius"], 0.05, `${where}.hitRadius`)),
      gravityPower: sbNumber(raw["gravityPower"], 0, `${where}.gravityPower`),
      gravityDir: sbVec3(raw["gravityDir"], [0, -1, 0], `${where}.gravityDir`),
      stiffness: Math.max(0, sbNumber(raw["springStiffness"], 0.5, `${where}.springStiffness`)),
      dragForce: Math.min(1, Math.max(0, sbNumber(raw["springDamping"], 0.5, `${where}.springDamping`))),
    };
  };

  const collidersValue = rig["colliders"];
  const colliderObjs: unknown[] = Array.isArray(collidersValue) ? collidersValue : isRecord(collidersValue) ? [collidersValue] : [];
  const colliders: Vrm1Collider[] = [];
  const colliderGroupIds: number[] = [];
  colliderObjs.forEach((entry, i) => {
    if (!isRecord(entry)) sbFail(`colliders[${i}] inválido`);
    if (Array.isArray(entry["spheres"])) {
      entry["spheres"].forEach((s: unknown, j: number) => {
        const w = `colliders[${i}].spheres[${j}]`;
        if (!isRecord(s)) sbFail(`${w} inválido`);
        const node = typeof s["node"] === "number" && Number.isInteger(s["node"]) && s["node"] >= 0 ? s["node"] : 0;
        checkNode(node, w);
        colliders.push({
          node,
          shape: {
            sphere: {
              offset: sbVec3(s["offset"], [0, 0, 0], `${w}.offset`),
              radius: Math.max(0, sbNumber(s["radius"], 0, `${w}.radius`)),
              inside: false,
            },
          },
        });
        colliderGroupIds.push(typeof s["group"] === "number" && Number.isInteger(s["group"]) && s["group"] >= 0 ? s["group"] : 0);
      });
    }
    if (Array.isArray(entry["capsules"])) {
      entry["capsules"].forEach((c: unknown, j: number) => {
        const w = `colliders[${i}].capsules[${j}]`;
        if (!isRecord(c)) sbFail(`${w} inválido`);
        const node = typeof c["node"] === "number" && Number.isInteger(c["node"]) && c["node"] >= 0 ? c["node"] : 0;
        checkNode(node, w);
        colliders.push({
          node,
          shape: {
            capsule: {
              offset: sbVec3(c["from"], [0, 0, 0], `${w}.from`),
              tail: sbVec3(c["to"], [0, 0, 0], `${w}.to`),
              radius: Math.max(0, sbNumber(c["radius"], 0, `${w}.radius`)),
              inside: false,
            },
          },
        });
        colliderGroupIds.push(typeof c["group"] === "number" && Number.isInteger(c["group"]) && c["group"] >= 0 ? c["group"] : 0);
      });
    }
  });

  const springs: Vrm1Spring[] = [];
  const springGroupIds: number[][] = [];
  const referenced: number[] = [...colliderGroupIds];
  const groupsRaw = Array.isArray(rig["groups"]) ? (rig["groups"] as unknown[]) : [];
  groupsRaw.forEach((entry, i) => {
    if (!isRecord(entry)) sbFail(`groups[${i}] inválido`);
    let center: number | undefined;
    if (entry["center"] !== undefined && entry["center"] !== null) {
      center = legacyJoint(entry["center"], `groups[${i}].center`).node;
      checkNode(center, `groups[${i}].center`);
    }
    const joints = (Array.isArray(entry["joints"]) ? (entry["joints"] as unknown[]) : []).map((raw, j) => {
      const joint = legacyJoint(raw, `groups[${i}].joints[${j}]`);
      checkNode(joint.node, `groups[${i}].joints[${j}]`);
      return joint;
    });
    const ids = (Array.isArray(entry["colliderGroups"]) ? (entry["colliderGroups"] as unknown[]) : []).filter(
      (v): v is number => typeof v === "number" && Number.isInteger(v) && v >= 0,
    );
    referenced.push(...ids);
    springGroupIds.push(ids);
    springs.push({
      name: sbName(entry["name"]) ?? `group_${i}`,
      joints,
      colliderGroups: [],
      ...(center !== undefined ? { center } : {}),
    });
  });

  const sortedIds = [...new Set(referenced)].sort((a, b) => a - b);
  const colliderGroups: Vrm1ColliderGroup[] = sortedIds.map((id) => ({
    name: `group_${id}`,
    colliders: colliderGroupIds.flatMap((g, idx) => (g === id ? [idx] : [])),
  }));
  springs.forEach((spring, i) => {
    spring.colliderGroups = springGroupIds[i].flatMap((id) => {
      const pos = sortedIds.indexOf(id);
      return pos >= 0 ? [pos] : [];
    });
  });
  return { specVersion: SPRING_BONE_SPEC_VERSION, colliders, colliderGroups, springs };
}

/** Lê um bloco `VRMC_springBone` (oficial ou legado). */
export function parseSpringBone(
  raw: unknown,
  nodes: ReadonlyArray<{ children?: number[] }>,
  warnings: string[],
): Vrm1SpringBone {
  if (!isRecord(raw)) sbFail("VRMC_springBone inválido (objeto esperado)");
  if ("secondaryRig" in raw) {
    const rig = raw["secondaryRig"];
    if (!isRecord(rig)) sbFail("VRMC_springBone.secondaryRig inválido");
    return parseLegacySpringBone(rig, nodes.length, warnings);
  }
  return parseOfficialSpringBone(raw, nodes, warnings);
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
    ? parseSpringBone(vrmRaw["secondary"] ?? gltf.extensions?.["VRMC_springBone"], gltf.nodes ?? [], warnings)
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
