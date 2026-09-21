/**
 * VRM 1.0 interoperability at the browser boundary.
 *
 * This module is intentionally a small, deterministic GLB container adapter.
 * It validates untrusted files before the renderer sees them and preserves the
 * binary chunk byte-for-byte when an ANIGO VRM envelope is exported. The Rust
 * `anigo-vrm` crate is the authoritative desktop implementation; keeping the
 * same checks here gives the web preview the same fail-fast behaviour.
 */

export const GLB_MAGIC = 0x46546c67;
export const GLB_VERSION = 2;
export const JSON_CHUNK = 0x4e4f534a;
export const BIN_CHUNK = 0x004e4942;
export const VRM_EXTENSION = "VRMC_vrm";
export const MTOON_EXTENSION = "VRMC_materials_mtoon";
export const SPRING_BONE_EXTENSION = "VRMC_springBone";
export const NODE_CONSTRAINT_EXTENSION = "VRMC_node_constraint";

export type VrmIssueSeverity = "error" | "warning";
export interface VrmIssue {
  severity: VrmIssueSeverity;
  code: string;
  path: string;
  message: string;
}
export interface VrmValidationReport {
  valid: boolean;
  isVrm: boolean;
  specVersion: string | null;
  issues: VrmIssue[];
  nodeCount: number;
  meshCount: number;
  materialCount: number;
}

export interface VrmMeta {
  title: string;
  version: string;
  author: string;
  contactInformation?: string;
  references?: string[];
}
export interface VrmHumanoidBone { bone: string; node: number }
export interface VrmExpression { name: string; weight: number; presetName?: string }
export interface VrmExportOptions {
  meta: VrmMeta;
  humanoid: VrmHumanoidBone[];
  expressions?: VrmExpression[];
  mtoonMaterials?: Array<{
    material: number;
    shadeColorFactor?: [number, number, number, number];
    shadingShiftFactor?: number;
    shadingToonyFactor?: number;
    outlineWidthMode?: string;
    outlineWidthFactor?: number;
    outlineColorFactor?: [number, number, number, number];
  }>;
  springBone?: Record<string, unknown>;
  nodeConstraint?: Record<string, unknown>;
}

export interface VrmImportSummary {
  meta: VrmMeta;
  specVersion: string;
  nodeCount: number;
  meshCount: number;
  materialCount: number;
  humanoidBones: VrmHumanoidBone[];
  expressions: VrmExpression[];
  hasMtoon: boolean;
  hasSpringBone: boolean;
  hasNodeConstraint: boolean;
}

export class VrmInteropError extends Error {
  readonly code: string;
  readonly recoverable = true;
  constructor(code: string, message: string) {
    super(`[ANIGO VRM ${code}] ${message}`);
    this.name = "VrmInteropError";
    this.code = code;
  }
}

export interface GlbJsonDocument {
  json: Record<string, unknown>;
  bin: Uint8Array;
}

function object(value: unknown, path: string): Record<string, unknown> | null {
  if (typeof value !== "object" || value === null || Array.isArray(value)) return null;
  return value as Record<string, unknown>;
}
function array(value: unknown): unknown[] | null { return Array.isArray(value) ? value : null; }
function issue(issues: VrmIssue[], severity: VrmIssueSeverity, code: string, path: string, message: string): void {
  issues.push({ severity, code, path, message });
}

function readU32(bytes: Uint8Array, offset: number): number {
  if (offset < 0 || offset + 4 > bytes.byteLength) {
    throw new VrmInteropError("TRUNCATED", `GLB requires four bytes at offset ${offset}`);
  }
  return new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength).getUint32(offset, true);
}

/** Parses only the GLB container. Geometry remains owned by the loader. */
export function parseGlbJson(input: ArrayBuffer | Uint8Array): GlbJsonDocument {
  const bytes = input instanceof Uint8Array ? input : new Uint8Array(input);
  if (bytes.byteLength < 12) throw new VrmInteropError("TRUNCATED", "arquivo GLB menor que o cabeçalho de 12 bytes");
  if (readU32(bytes, 0) !== GLB_MAGIC) throw new VrmInteropError("BAD_MAGIC", "o arquivo não é um GLB glTF 2.0");
  if (readU32(bytes, 4) !== GLB_VERSION) throw new VrmInteropError("BAD_VERSION", "apenas GLB versão 2 é suportado");
  const declared = readU32(bytes, 8);
  if (declared !== bytes.byteLength || declared < 20) {
    throw new VrmInteropError("BAD_LENGTH", `o cabeçalho declara ${declared} bytes, mas o arquivo tem ${bytes.byteLength}`);
  }
  let offset = 12;
  let jsonBytes: Uint8Array | null = null;
  let bin = new Uint8Array(0);
  while (offset < declared) {
    if (offset + 8 > declared) throw new VrmInteropError("BAD_CHUNK", `chunk incompleto no offset ${offset}`);
    const length = readU32(bytes, offset);
    const type = readU32(bytes, offset + 4);
    const start = offset + 8;
    const end = start + length;
    if (end > declared) throw new VrmInteropError("BAD_CHUNK", `chunk ultrapassa o tamanho do GLB no offset ${offset}`);
    if (type === JSON_CHUNK && jsonBytes === null) jsonBytes = bytes.slice(start, end);
    if (type === BIN_CHUNK && bin.byteLength === 0) bin = bytes.slice(start, end);
    offset = end;
  }
  if (!jsonBytes) throw new VrmInteropError("MISSING_JSON", "GLB sem chunk JSON");
  let json: unknown;
  try {
    const text = new TextDecoder().decode(jsonBytes).replace(/[\u0000 ]+$/g, "");
    json = JSON.parse(text);
  } catch (error) {
    throw new VrmInteropError("BAD_JSON", String(error));
  }
  const root = object(json, "root");
  if (!root) throw new VrmInteropError("BAD_JSON", "a raiz JSON do GLB precisa ser um objeto");
  return { json: root, bin };
}

/** Validates glTF/VRM JSON and returns all actionable issues. */
export function validateVrmJson(value: unknown, requireVrm = true): VrmValidationReport {
  const issues: VrmIssue[] = [];
  const root = object(value, "root");
  if (!root) {
    return { valid: false, isVrm: false, specVersion: null, issues: [{ severity: "error", code: "ROOT_OBJECT", path: "/", message: "a raiz do documento deve ser um objeto" }], nodeCount: 0, meshCount: 0, materialCount: 0 };
  }
  const asset = object(root.asset, "/asset");
  if (asset?.version !== "2.0") issue(issues, "error", "GLTF_VERSION", "/asset/version", "asset.version deve ser '2.0'");
  const nodeCount = array(root.nodes)?.length ?? 0;
  const meshCount = array(root.meshes)?.length ?? 0;
  const materialCount = array(root.materials)?.length ?? 0;
  if (nodeCount === 0) issue(issues, "error", "NO_NODES", "/nodes", "o documento precisa de ao menos um node");
  if (meshCount === 0) issue(issues, "warning", "NO_MESHES", "/meshes", "o documento não possui malha para visualizar");
  if (!array(root.scenes)) issue(issues, "error", "NO_SCENES", "/scenes", "glTF precisa declarar scenes");

  const extensions = object(root.extensions, "/extensions");
  const vrm = object(extensions?.[VRM_EXTENSION], `/extensions/${VRM_EXTENSION}`);
  const specVersion = typeof vrm?.specVersion === "string" ? vrm.specVersion : null;
  if (requireVrm && !vrm) issue(issues, "error", "MISSING_VRM_EXTENSION", `/extensions/${VRM_EXTENSION}`, "VRM 1.0 requer a extensão VRMC_vrm");
  if (requireVrm && specVersion !== "1.0") issue(issues, "error", "VRM_SPEC_VERSION", `/extensions/${VRM_EXTENSION}/specVersion`, "specVersion deve ser '1.0'");
  if (vrm) {
    validateMeta(vrm, issues);
    validateHumanoid(vrm, nodeCount, issues);
    validateExpressions(vrm, issues);
  }
  for (const extension of [MTOON_EXTENSION, SPRING_BONE_EXTENSION, NODE_CONSTRAINT_EXTENSION]) {
    const payload = extensions?.[extension];
    if (payload !== undefined && object(payload, `/extensions/${extension}`)?.specVersion !== "1.0") {
      issue(issues, "error", "EXTENSION_VERSION", `/extensions/${extension}/specVersion`, "a extensão deve declarar specVersion '1.0'");
    }
  }
  const isVrm = Boolean(vrm && specVersion === "1.0");
  return { valid: !issues.some((entry) => entry.severity === "error") && (!requireVrm || isVrm), isVrm, specVersion, issues, nodeCount, meshCount, materialCount };
}

function validateMeta(vrm: Record<string, unknown>, issues: VrmIssue[]): void {
  const meta = object(vrm.meta, `/extensions/${VRM_EXTENSION}/meta`);
  if (!meta) { issue(issues, "error", "MISSING_META", `/extensions/${VRM_EXTENSION}/meta`, "VRM requer metadados"); return; }
  for (const name of ["name", "version"]) {
    if (typeof meta[name] !== "string" || meta[name].length === 0) issue(issues, "error", "META_FIELD", `/extensions/${VRM_EXTENSION}/meta/${name}`, "campo obrigatório não vazio");
  }
  if (!Array.isArray(meta.authors) || meta.authors.length === 0 || meta.authors.some((author) => typeof author !== "string" || author.length === 0)) {
    issue(issues, "error", "META_AUTHORS", `/extensions/${VRM_EXTENSION}/meta/authors`, "authors deve conter ao menos um nome");
  }
}
function validateHumanoid(vrm: Record<string, unknown>, nodeCount: number, issues: VrmIssue[]): void {
  const humanoid = object(vrm.humanoid, `/extensions/${VRM_EXTENSION}/humanoid`);
  const bones = array(humanoid?.humanBones);
  if (!bones) { issue(issues, "error", "MISSING_HUMAN_BONES", `/extensions/${VRM_EXTENSION}/humanoid/humanBones`, "humanBones deve ser uma lista"); return; }
  const seen = new Set<string>();
  bones.forEach((raw, index) => {
    const bone = object(raw, `humanBones[${index}]`);
    const name = bone?.bone;
    const node = bone?.node;
    if (typeof name !== "string" || typeof node !== "number" || !Number.isInteger(node)) {
      issue(issues, "error", "HUMANOID_ENTRY", `/extensions/${VRM_EXTENSION}/humanoid/humanBones/${index}`, "cada entrada requer bone e node inteiro");
      return;
    }
    if (seen.has(name)) issue(issues, "error", "DUPLICATE_HUMANOID_BONE", `/extensions/${VRM_EXTENSION}/humanoid/humanBones/${index}/bone`, "bone duplicado");
    seen.add(name);
    if (node < 0 || node >= nodeCount) issue(issues, "error", "HUMANOID_NODE", `/extensions/${VRM_EXTENSION}/humanoid/humanBones/${index}/node`, "node fora do array nodes");
  });
  for (const required of ["hips", "spine", "head"]) if (!seen.has(required)) issue(issues, "warning", "INCOMPLETE_HUMANOID", `/extensions/${VRM_EXTENSION}/humanoid/humanBones`, `osso recomendado ausente: ${required}`);
}
function validateExpressions(vrm: Record<string, unknown>, issues: VrmIssue[]): void {
  if (vrm.expressions === undefined) return;
  const expressions = object(vrm.expressions, `/extensions/${VRM_EXTENSION}/expressions`);
  if (!expressions) { issue(issues, "error", "EXPRESSIONS_OBJECT", `/extensions/${VRM_EXTENSION}/expressions`, "expressions deve ser objeto"); return; }
  for (const group of ["preset", "custom"]) if (expressions[group] !== undefined && !object(expressions[group], group)) issue(issues, "error", "EXPRESSIONS_GROUP", `/extensions/${VRM_EXTENSION}/expressions/${group}`, "grupo de expressões deve ser objeto");
}

/**
 * Converts a JSON glTF 2.0 document to GLB for the shared preview decoder.
 *
 * A browser file picker cannot resolve arbitrary relative URLs after the file
 * leaves its directory, so callers may provide the selected sibling resources
 * by URI. Data URIs work without any extra files. Buffer views are rebased into
 * one BIN chunk while the JSON scene graph remains otherwise unchanged.
 */
export function gltfJsonToGlb(
  input: string | ArrayBuffer | Uint8Array,
  resources: ReadonlyMap<string, ArrayBuffer> = new Map()
): ArrayBuffer {
  const parsed = typeof input === "string" ? JSON.parse(input) as unknown : new TextDecoder().decode(input instanceof Uint8Array ? input : new Uint8Array(input));
  const source = object(typeof parsed === "string" ? JSON.parse(parsed) : parsed, "/");
  if (!source) throw new VrmInteropError("BAD_JSON", "a raiz do glTF precisa ser um objeto");
  const root = cloneJson(source);
  const buffers = array(root.buffers) ?? [];
  const payloads: Uint8Array[] = [];
  const baseOffsets: number[] = [];
  let total = 0;
  for (let index = 0; index < buffers.length; index += 1) {
    const descriptor = object(buffers[index], `/buffers/${index}`);
    if (!descriptor) throw new VrmInteropError("BAD_BUFFER", `buffers[${index}] precisa ser um objeto`);
    const uri = typeof descriptor.uri === "string" ? descriptor.uri : null;
    let payload: Uint8Array;
    if (uri?.startsWith("data:")) {
      payload = decodeDataUri(uri);
    } else if (uri) {
      const resource = resources.get(uri) ?? resources.get(uri.split("/").pop() ?? uri);
      if (!resource) throw new VrmInteropError("EXTERNAL_RESOURCE", `buffer externo não selecionado: ${uri}`);
      payload = new Uint8Array(resource);
    } else {
      throw new VrmInteropError("MISSING_BUFFER_URI", `buffers[${index}] não possui uri; selecione o GLB correspondente`);
    }
    const declared = typeof descriptor.byteLength === "number" ? descriptor.byteLength : payload.byteLength;
    if (payload.byteLength < declared) throw new VrmInteropError("BUFFER_SHORT", `buffer ${index} declara ${declared} bytes, mas recebeu ${payload.byteLength}`);
    baseOffsets[index] = total;
    payloads.push(payload);
    total = padded(total + payload.byteLength);
  }
  const bin = new Uint8Array(total);
  payloads.forEach((payload, index) => bin.set(payload, baseOffsets[index]));
  const views = array(root.bufferViews) ?? [];
  views.forEach((raw, index) => {
    const view = object(raw, `/bufferViews/${index}`);
    if (!view) return;
    const bufferIndex = typeof view.buffer === "number" ? view.buffer : 0;
    const localOffset = typeof view.byteOffset === "number" ? view.byteOffset : 0;
    view.buffer = 0;
    view.byteOffset = (baseOffsets[bufferIndex] ?? 0) + localOffset;
  });
  root.buffers = [{ byteLength: bin.byteLength }];
  return writeGlbJson({ json: root, bin });
}

function decodeDataUri(uri: string): Uint8Array {
  const comma = uri.indexOf(",");
  if (comma < 0) throw new VrmInteropError("BAD_DATA_URI", "data URI sem payload");
  const header = uri.slice(0, comma);
  const payload = uri.slice(comma + 1);
  if (/;base64$/i.test(header)) {
    if (typeof atob !== "function") throw new VrmInteropError("BASE64_UNAVAILABLE", "este ambiente não oferece decodificação base64");
    const binary = atob(payload);
    return Uint8Array.from(binary, (character) => character.charCodeAt(0));
  }
  return new TextEncoder().encode(decodeURIComponent(payload));
}

export function validateVrmGlb(input: ArrayBuffer | Uint8Array): VrmValidationReport {
  return validateVrmJson(parseGlbJson(input).json, true);
}
export function validateGltfGlb(input: ArrayBuffer | Uint8Array): VrmValidationReport {
  return validateVrmJson(parseGlbJson(input).json, false);
}

function cloneJson<T>(value: T): T { return JSON.parse(JSON.stringify(value)) as T; }
function appendExtension(root: Record<string, unknown>, name: string): void {
  const used = Array.isArray(root.extensionsUsed) ? root.extensionsUsed as unknown[] : [];
  if (!used.includes(name)) used.push(name);
  root.extensionsUsed = used;
}
function padded(value: number): number { return (value + 3) & ~3; }

/** Encodes a modified JSON document while retaining the source BIN bytes. */
export function writeGlbJson(document: GlbJsonDocument): ArrayBuffer {
  const jsonBytes = new TextEncoder().encode(JSON.stringify(document.json));
  const jsonLength = padded(jsonBytes.byteLength);
  const binLength = padded(document.bin.byteLength);
  const total = 12 + 8 + jsonLength + 8 + binLength;
  const output = new Uint8Array(total);
  const view = new DataView(output.buffer);
  view.setUint32(0, GLB_MAGIC, true); view.setUint32(4, GLB_VERSION, true); view.setUint32(8, total, true);
  view.setUint32(12, jsonLength, true); view.setUint32(16, JSON_CHUNK, true); output.set(jsonBytes, 20);
  output.fill(0x20, 20 + jsonBytes.byteLength, 20 + jsonLength);
  const binHeader = 20 + jsonLength;
  view.setUint32(binHeader, binLength, true); view.setUint32(binHeader + 4, BIN_CHUNK, true);
  output.set(document.bin, binHeader + 8);
  return output.buffer;
}

/** Builds a VRM 1.0 envelope around an existing glTF 2.0 asset. */
export function exportVrmGlb(base: ArrayBuffer | Uint8Array, options: VrmExportOptions): ArrayBuffer {
  const document = parseGlbJson(base);
  const baseReport = validateVrmJson(document.json, false);
  if (!baseReport.valid) throw new VrmInteropError("INVALID_GLTF", baseReport.issues.filter((entry) => entry.severity === "error").map((entry) => `${entry.code}: ${entry.message}`).join("; "));
  for (const bone of options.humanoid) if (!Number.isInteger(bone.node) || bone.node < 0 || bone.node >= baseReport.nodeCount) throw new VrmInteropError("HUMANOID_NODE", `osso ${bone.bone} aponta para node ${bone.node} inválido`);
  const root = cloneJson(document.json);
  const extensions = object(root.extensions, "/extensions") ?? {};
  root.extensions = extensions;
  const existingVrm = object(extensions[VRM_EXTENSION], "/extensions/VRMC_vrm") ?? {};
  extensions[VRM_EXTENSION] = existingVrm;
  Object.assign(existingVrm, {
    specVersion: "1.0",
    meta: { name: options.meta.title, version: options.meta.version, authors: [options.meta.author], ...(options.meta.contactInformation ? { contactInformation: options.meta.contactInformation } : {}), references: options.meta.references ?? [] },
    humanoid: { humanBones: options.humanoid.map((bone) => ({ bone: bone.bone, node: bone.node })) },
  });
  if (options.expressions && options.expressions.length > 0) {
    existingVrm.expressions = { preset: Object.fromEntries(options.expressions.map((expression) => [expression.name, { overrideBlink: Math.max(0, Math.min(1, expression.weight)) }])), custom: {} };
  }
  appendExtension(root, VRM_EXTENSION);
  if (options.mtoonMaterials && options.mtoonMaterials.length > 0) {
    extensions[MTOON_EXTENSION] = { specVersion: "1.0", materials: options.mtoonMaterials.filter((material) => material.material >= 0 && material.material < baseReport.materialCount) };
    appendExtension(root, MTOON_EXTENSION);
  }
  if (options.springBone) { extensions[SPRING_BONE_EXTENSION] = { specVersion: "1.0", ...options.springBone }; appendExtension(root, SPRING_BONE_EXTENSION); }
  if (options.nodeConstraint) { extensions[NODE_CONSTRAINT_EXTENSION] = { specVersion: "1.0", ...options.nodeConstraint }; appendExtension(root, NODE_CONSTRAINT_EXTENSION); }
  const result = writeGlbJson({ json: root, bin: document.bin });
  const report = validateVrmGlb(result);
  if (!report.valid) throw new VrmInteropError("INVALID_EXPORT", report.issues.map((entry) => entry.message).join("; "));
  return result;
}

export function summarizeVrmJson(value: unknown): VrmImportSummary {
  const report = validateVrmJson(value, true);
  if (!report.valid) throw new VrmInteropError("INVALID_VRM", report.issues.filter((entry) => entry.severity === "error").map((entry) => entry.message).join("; "));
  const root = object(value, "/") ?? {};
  const vrm = object(object(root.extensions, "/extensions")?.[VRM_EXTENSION], "/extensions/VRMC_vrm") ?? {};
  const meta = object(vrm.meta, "meta") ?? {};
  const authors = Array.isArray(meta.authors) ? meta.authors : [];
  const humanoid = array(object(vrm.humanoid, "humanoid")?.humanBones) ?? [];
  const expressions = object(vrm.expressions, "expressions");
  const preset = object(expressions?.preset, "preset") ?? {};
  return {
    meta: { title: typeof meta.name === "string" ? meta.name : "ANIGO Character", version: typeof meta.version === "string" ? meta.version : "1.0", author: typeof authors[0] === "string" ? authors[0] : "ANIGO Studio", ...(typeof meta.contactInformation === "string" ? { contactInformation: meta.contactInformation } : {}), references: Array.isArray(meta.references) ? meta.references.filter((item): item is string => typeof item === "string") : [] },
    specVersion: report.specVersion ?? "1.0", nodeCount: report.nodeCount, meshCount: report.meshCount, materialCount: report.materialCount,
    humanoidBones: humanoid.flatMap((raw) => { const item = object(raw, "bone"); return typeof item?.bone === "string" && typeof item.node === "number" ? [{ bone: item.bone, node: item.node }] : []; }),
    expressions: Object.entries(preset).map(([name, raw]) => ({ name, presetName: name, weight: typeof object(raw, name)?.overrideBlink === "number" ? object(raw, name)?.overrideBlink as number : 0 })),
    hasMtoon: Boolean(object(root.extensions, "extensions")?.[MTOON_EXTENSION]), hasSpringBone: Boolean(object(root.extensions, "extensions")?.[SPRING_BONE_EXTENSION]), hasNodeConstraint: Boolean(object(root.extensions, "extensions")?.[NODE_CONSTRAINT_EXTENSION]),
  };
}

export async function importVrmFile(file: File): Promise<{ bytes: ArrayBuffer; report: VrmValidationReport; summary: VrmImportSummary }> {
  const source = await file.arrayBuffer();
  const bytes = /\.gltf$/i.test(file.name) ? gltfJsonToGlb(await file.text()) : source;
  const document = parseGlbJson(bytes);
  const report = validateVrmJson(document.json, true);
  if (!report.valid) throw new VrmInteropError("INVALID_VRM", report.issues.filter((entry) => entry.severity === "error").map((entry) => `${entry.code}: ${entry.message}`).join("; "));
  return { bytes, report, summary: summarizeVrmJson(document.json) };
}
