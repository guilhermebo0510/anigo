/**
 * ANIGO — Contrato do renderer v1 (P0 "Consolidar o renderer").
 *
 * O arquivo congelado `contracts/fixtures/render_contract_v1.json` é a **única**
 * definição de shaders, buffers/uniforms, câmera, passes e do toon ramp. O
 * renderer Rust (`crates/anigo-renderer/src/render_contract.rs`) e o viewport
 * (este módulo) leem o mesmo documento, então divergência entre eles vira erro
 * de teste em vez de diferença visual silenciosa.
 *
 * O que este módulo expõe:
 *   - `RENDER_CONTRACT` validado (versão, campos obrigatórios, hashes);
 *   - helpers de layout (vertex buffer, bytes de cada uniform, ordem de grupos);
 *   - o grafo de passes e o estado de pipeline de cada um;
 *   - a especificação do toon ramp (linhas/thresholds) usada para gerar a textura.
 *
 * Os shaders de produção vivem em `crates/anigo-renderer/shaders/` — o Rust usa
 * `include_str!` e o viewport importa com `?raw`. Não existe segunda cópia
 * (§2 das Regras Invioláveis: uma definição de produção).
 */

import { RENDER_CONTRACT_DATA } from "./render_contract_data.v1";

export const RENDER_CONTRACT_VERSION = 1;

/** Papel de um shader no repositório. */
export type ShaderRole = "production" | "fallback_webgl2" | "library";

export interface ShaderSourceV1 {
  name: string;
  /** Caminho relativo à raiz do repositório (fonte canônica). */
  path: string;
  language: "wgsl" | "glsl";
  role: ShaderRole;
  entry_points: Record<string, string>;
  lines: number;
  /** FNV-1a-64 do conteúdo — o mesmo algoritmo de `ids.rs`/`fnv1a64`. */
  fnv1a64: string;
}

export interface VertexAttributeV1 {
  shader_location: number;
  offset: number;
  format: string;
  name: string;
}

export interface UniformFieldV1 {
  name: string;
  kind: string;
  offset: number;
  size: number;
  meaning?: string;
}

export interface UniformLayoutV1 {
  /** Nome da struct no WGSL (`CameraUniform`, `BonePalette`, ...). */
  struct?: string;
  /** "uniform" | "storage_read" | "vertex_and_storage_read". */
  address_space: string;
  size: number;
  note?: string;
  fields: UniformFieldV1[];
}

export interface BindGroupEntryV1 {
  binding: number;
  kind: string;
  stages: string[];
  declaration: string;
}

export interface BindGroupV1 {
  name: string;
  group: number;
  entries: BindGroupEntryV1[];
}

export interface PassSpecV1 {
  order: number;
  name: string;
  kind: "render" | "compute";
  shader: string;
  vertex_entry?: string;
  fragment_entry?: string;
  compute_entry?: string;
  bind_group: string;
  cull_mode?: string;
  depth_write?: boolean;
  depth_compare?: string;
  depth_bias?: { constant: number; slope_scale: number; clamp: number };
  blend?: "none" | "src_alpha_one_minus_src_alpha";
  write_mask?: string;
  workgroup_size?: number;
  only_when?: string;
}

export interface ToonRampRowV1 {
  name: string;
  kind: "identity" | "steps";
  steps?: Array<{ threshold: number; value: number }>;
  base?: number;
}

export interface DiagnosticCodeSpecV1 {
  code: string;
  severity: "info" | "warning" | "error";
}

/**
 * P1-04: skinning linear (LBS) — paleta de ossos compartilhada com o Rust.
 *
 * `joints_location`/`weights_location` são os atributos de vértice que o layout
 * de 72 B carrega, e `block_markers` delimita o trecho que precisa ser
 * **idêntico** entre os shaders WGSL (o checker `scripts/check_wgsl.mjs`
 * compara byte a byte).
 */
export interface SkinningSpecV1 {
  algorithm: string;
  joint_count: number;
  max_influences: number;
  matrices_per_joint: number;
  palette_uniform: string;
  palette_bytes: number;
  joints_location: number;
  weights_location: number;
  weights_normalization: string;
  unskinned_fallback: string;
  index_clamp: string;
  block_markers: string[];
}

export interface RenderContractV1 {
  version: number;
  /** P1-02: vocabulário de diagnóstico compartilhado com o Rust. */
  diagnostics?: { note: string; codes: DiagnosticCodeSpecV1[] };
  /** P1-04: skinning (paleta de ossos + atributos de vértice). */
  skinning?: SkinningSpecV1;
  /** P1-03: códigos de validação de malha antes de criar buffers. */
  mesh_validation?: {
    note: string;
    stride_bytes: number;
    codes: Array<{ code: string; meaning: string }>;
  };
  shaders: ShaderSourceV1[];
  vertex_layout: { stride: number; step_mode: string; attributes: VertexAttributeV1[] };
  uniforms: Record<string, UniformLayoutV1>;
  bind_groups: BindGroupV1[];
  passes: PassSpecV1[];
  targets: {
    offscreen_color_format: string;
    viewport_color_format_policy: string;
    depth_format: string;
    msaa_samples: number;
    color_space: string;
    resolve_to_swapchain: boolean;
    clear: { source: string };
  };
  camera: {
    projection: string;
    clip_depth: string;
    matrix_layout: string;
    up_axis: string;
    fov_y_degrees_default: number;
    z_near_default: number;
    z_far_default: number;
    model_from: string;
    uniform: string;
  };
  toon_ramp: {
    width: number;
    height: number;
    format: string;
    mag_filter: string;
    min_filter: string;
    address_mode: string;
    rows: ToonRampRowV1[];
    /** Impressão digital dos bytes que a textura precisa ter (congelada). */
    bytes_len: number;
    bytes_fnv1a64: string;
    sample_u8: number[];
  };
  reference_frame: {
    camera: {
      eye: [number, number, number];
      target: [number, number, number];
      up: [number, number, number];
      fov_y_degrees: number;
      fov_y_radians: number;
      aspect: number;
      z_near: number;
      z_far: number;
    };
    model: { translation: [number, number, number]; scale: [number, number, number]; rotation_xyzw: number[] };
    light: Record<string, number | number[]>;
    material: Record<string, number | number[]>;
    expected: {
      view_proj: number[];
      model_matrix: number[];
      normal_matrix: number[];
      camera_uniform: number[];
      light_uniform: number[];
      material_uniform: number[];
      outline_uniform: number[];
    };
  };
}

export class RenderContractError extends Error {
  readonly recoverable = true;
  readonly code: string;

  constructor(code: string, message: string) {
    super(message);
    this.code = code;
    this.name = "RenderContractError";
  }
}

const UNIFORM_BLOCK_ALIGNMENT = 16;
const UNIFORM_MIN_ALIGNMENT = 4;

/** Espaços de memória declarados no contrato (`address_space`). */
export const UNIFORM_ADDRESS_SPACES = ["uniform", "storage_read", "vertex_and_storage_read"] as const;

/** Validates the frozen contract (shape only — values are contract data). */
export function readRenderContract(value: unknown = RENDER_CONTRACT_DATA): RenderContractV1 {
  if (typeof value !== "object" || value === null) {
    throw new RenderContractError("not_object", "render contract precisa ser um objeto");
  }
  const contract = value as RenderContractV1;
  if (contract.version !== RENDER_CONTRACT_VERSION) {
    throw new RenderContractError(
      "unsupported_version",
      `render contract na versão ${contract.version}, esperado ${RENDER_CONTRACT_VERSION}`
    );
  }
  if (!Array.isArray(contract.shaders) || contract.shaders.length === 0) {
    throw new RenderContractError("missing_shaders", "render contract sem shaders");
  }
  const production = contract.shaders.filter((shader) => shader.role === "production");
  if (production.length === 0) {
    throw new RenderContractError("missing_shaders", "nenhum shader de produção declarado");
  }
  for (const shader of contract.shaders) {
    if (!shader.path.endsWith(shader.language === "wgsl" ? ".wgsl" : ".glsl")) {
      throw new RenderContractError("bad_shader_path", `${shader.name}: extensão não bate com a linguagem`);
    }
    if (!/^[0-9a-f]{16}$/.test(shader.fnv1a64)) {
      throw new RenderContractError("bad_shader_hash", `${shader.name}: hash precisa ser hex de 16 dígitos`);
    }
  }
  if (!Array.isArray(contract.passes) || contract.passes.length === 0) {
    throw new RenderContractError("missing_passes", "render contract sem passes");
  }
  for (const pass of contract.passes) {
    if (!contract.shaders.some((shader) => shader.name === pass.shader)) {
      throw new RenderContractError("unknown_shader", `passe '${pass.name}' usa shader desconhecido '${pass.shader}'`);
    }
    if (!contract.bind_groups.some((group) => group.name === pass.bind_group)) {
      throw new RenderContractError("unknown_bind_group", `passe '${pass.name}' usa bind group '${pass.bind_group}'`);
    }
  }
  for (const [name, layout] of Object.entries(contract.uniforms)) {
    if (!(UNIFORM_ADDRESS_SPACES as readonly string[]).includes(layout.address_space)) {
      throw new RenderContractError(
        "bad_address_space",
        `bloco '${name}' declara address_space '${layout.address_space}' desconhecido`
      );
    }
    // Blocos do espaço `uniform` do WGSL exigem alinhamento de 16 B; os demais
    // (storage/vertex) só precisam respeitar o alinhamento dos escalares —
    // `vertex_raw` tem 72 B justamente porque o WGSL o declara membro a membro.
    const alignment = layout.address_space === "uniform" ? UNIFORM_BLOCK_ALIGNMENT : UNIFORM_MIN_ALIGNMENT;
    if (layout.size % alignment !== 0) {
      throw new RenderContractError(
        "bad_uniform_size",
        `bloco '${name}' tem tamanho ${layout.size} (não múltiplo de ${alignment} para o espaço '${layout.address_space}')`
      );
    }
    for (const field of layout.fields) {
      if (field.offset + field.size > layout.size) {
        throw new RenderContractError("bad_uniform_field", `campo '${name}.${field.name}' passa do fim do bloco`);
      }
    }
  }
  validateSkinning(contract);
  return contract;
}

/**
 * P1-04: o skinning do contrato precisa apontar para blocos/bindings/atributos
 * que existem de verdade — paleta com tamanho diferente do uniform, binding
 * ausente ou atributo trocado viram erro de contrato (e não um vértice parado
 * na pose de repouso).
 */
function validateSkinning(contract: RenderContractV1): void {
  const skinning = contract.skinning;
  if (!skinning) {
    throw new RenderContractError("missing_skinning", "render contract sem a seção 'skinning'");
  }
  if (skinning.matrices_per_joint !== 16) {
    throw new RenderContractError(
      "bad_skinning",
      `skinning.matrices_per_joint = ${skinning.matrices_per_joint} (mat4 tem 16 floats)`
    );
  }
  if (skinning.joint_count < 1 || skinning.max_influences < 1 || skinning.max_influences > 4) {
    throw new RenderContractError(
      "bad_skinning",
      `skinning com ${skinning.joint_count} ossos e ${skinning.max_influences} influências por vértice`
    );
  }
  const palette = contract.uniforms[skinning.palette_uniform];
  if (!palette) {
    throw new RenderContractError(
      "bad_skinning",
      `skinning.palette_uniform '${skinning.palette_uniform}' não é um bloco do contrato`
    );
  }
  const expectedBytes = skinning.joint_count * skinning.matrices_per_joint * 4;
  if (palette.size !== expectedBytes || skinning.palette_bytes !== expectedBytes) {
    throw new RenderContractError(
      "bad_skinning",
      `paleta de ${skinning.joint_count} ossos precisa de ${expectedBytes} B ` +
        `(contrato diz ${skinning.palette_bytes} B, bloco '${palette.address_space}' tem ${palette.size} B)`
    );
  }
  for (const [locationKey, expectedName] of [
    ["joints_location", "joints"],
    ["weights_location", "weights"],
  ] as const) {
    const location = skinning[locationKey];
    const attribute = contract.vertex_layout.attributes.find(
      (candidate) => candidate.shader_location === location
    );
    if (!attribute || attribute.name !== expectedName) {
      throw new RenderContractError(
        "bad_skinning",
        `skinning.${locationKey} = ${location} não é o atributo '${expectedName}' do vertex layout`
      );
    }
  }
  const bound = contract.bind_groups.some((group) =>
    group.entries.some((entry) => entry.declaration.includes(`${skinning.palette_uniform}:`))
  );
  if (!bound) {
    throw new RenderContractError(
      "bad_skinning",
      `nenhum bind group declara '${skinning.palette_uniform}'`
    );
  }
}

export const RENDER_CONTRACT: RenderContractV1 = readRenderContract();

// ---------------------------------------------------------------------------
// Helpers usados pelo renderer
// ---------------------------------------------------------------------------

/** Layout do vertex buffer no formato aceito por `createRenderPipeline`. */
/** Códigos de diagnóstico declarados no contrato (`[]` se a seção faltar). */
export function diagnosticCodeSpecs(): DiagnosticCodeSpecV1[] {
  return RENDER_CONTRACT.diagnostics?.codes ?? [];
}

export function vertexBufferLayout(): {
  arrayStride: number;
  stepMode: "vertex";
  attributes: Array<{ shaderLocation: number; offset: number; format: string }>;
} {
  const layout = RENDER_CONTRACT.vertex_layout;
  return {
    arrayStride: layout.stride,
    stepMode: "vertex",
    attributes: layout.attributes.map((attribute) => ({
      shaderLocation: attribute.shader_location,
      offset: attribute.offset,
      format: attribute.format,
    })),
  };
}

/** Tamanho em bytes de um bloco uniform declarado no contrato. */
export function uniformSize(name: string): number {
  const layout = RENDER_CONTRACT.uniforms[name];
  if (!layout) throw new RenderContractError("unknown_uniform", `uniform '${name}' não está no contrato`);
  return layout.size;
}

/** Offset em bytes de um campo de um bloco uniform. */
export function uniformOffset(name: string, field: string): number {
  const layout = RENDER_CONTRACT.uniforms[name];
  const entry = layout?.fields.find((candidate) => candidate.name === field);
  if (!entry) throw new RenderContractError("unknown_uniform_field", `campo '${name}.${field}' não está no contrato`);
  return entry.offset;
}

/** Número de floats (f32) de um bloco uniform — usado pelos packers. */
export function uniformFloats(name: string): number {
  return uniformSize(name) / 4;
}

export function bindGroup(name: string): BindGroupV1 {
  const group = RENDER_CONTRACT.bind_groups.find((candidate) => candidate.name === name);
  if (!group) throw new RenderContractError("unknown_bind_group", `bind group '${name}' não está no contrato`);
  return group;
}

/** P1-04: especificação de skinning do contrato. */
export function skinning(): SkinningSpecV1 {
  const spec = RENDER_CONTRACT.skinning;
  if (!spec) throw new RenderContractError("missing_skinning", "render contract sem a seção 'skinning'");
  return spec;
}

/** Bytes da paleta de ossos (1536 = 24 × mat4). */
export function bonePaletteBytes(): number {
  return skinning().palette_bytes;
}

/** Floats da paleta de ossos. */
export function bonePaletteFloats(): number {
  return bonePaletteBytes() / 4;
}

/**
 * Binding do uniform da paleta dentro de um bind group — o número sai do
 * contrato (5 no cel, 2 no outline) em vez de literal no renderer.
 */
export function skinningBinding(groupName: string): number {
  const uniform = skinning().palette_uniform;
  const entry = bindGroup(groupName).entries.find((candidate) =>
    candidate.declaration.includes(`${uniform}:`)
  );
  if (!entry) {
    throw new RenderContractError(
      "bad_skinning",
      `bind group '${groupName}' não declara '${uniform}'`
    );
  }
  return entry.binding;
}

export function shaderOf(name: string): ShaderSourceV1 {
  const shader = RENDER_CONTRACT.shaders.find((candidate) => candidate.name === name);
  if (!shader) throw new RenderContractError("unknown_shader", `shader '${name}' não está no contrato`);
  return shader;
}

/** Passes de render na ordem canônica (compute fica fora do render pass). */
export function renderPasses(): PassSpecV1[] {
  return RENDER_CONTRACT.passes
    .filter((pass) => pass.kind === "render")
    .sort((a, b) => a.order - b.order);
}

export function computePasses(): PassSpecV1[] {
  return RENDER_CONTRACT.passes.filter((pass) => pass.kind === "compute");
}

export interface BlendStateLike {
  color: { srcFactor: string; dstFactor: string; operation: string };
  alpha: { srcFactor: string; dstFactor: string; operation: string };
}

/** Traduz `blend` do contrato para um `GPUBlendState` (ou `undefined`). */
export function blendStateOf(pass: PassSpecV1): BlendStateLike | undefined {
  if (pass.blend === "none" || pass.blend === undefined) return undefined;
  if (pass.blend === "src_alpha_one_minus_src_alpha") {
    return {
      color: { srcFactor: "src-alpha", dstFactor: "one-minus-src-alpha", operation: "add" },
      alpha: { srcFactor: "one", dstFactor: "one-minus-src-alpha", operation: "add" },
    };
  }
  throw new RenderContractError("unknown_blend", `blend '${pass.blend}' desconhecido`);
}

/** Converte um filtro/endereçamento do contrato para o valor do WebGPU. */
export function filterMode(value: string): "linear" | "nearest" {
  if (value === "linear" || value === "nearest") return value;
  throw new RenderContractError("unknown_filter", `filtro '${value}' desconhecido`);
}

export function addressMode(value: string): "clamp-to-edge" | "repeat" | "mirror-repeat" {
  if (value === "clamp_to_edge") return "clamp-to-edge";
  if (value === "repeat") return "repeat";
  if (value === "mirror_repeat") return "mirror-repeat";
  throw new RenderContractError("unknown_address_mode", `address mode '${value}' desconhecido`);
}

export function msaaSampleCount(): number {
  return RENDER_CONTRACT.targets.msaa_samples;
}

/** Formato do alvo offscreen do headless (o viewport usa o preferido da surface). */
export function offscreenColorFormat(): string {
  return RENDER_CONTRACT.targets.offscreen_color_format;
}

/** Formato do buffer de profundidade (o mesmo nos dois renderers). */
export function depthFormat(): string {
  return RENDER_CONTRACT.targets.depth_format;
}

/** Ordem de desenho dos passes de render (`order` crescente). */
export function renderPassOrder(): string[] {
  return renderPasses().map((pass) => pass.name);
}

/** FNV-1a de 64 bits sobre os bytes UTF-8 — mesma função do Rust e do gerador. */
export function fnv1a64(input: string | Uint8Array): string {
  const bytes = typeof input === "string" ? new TextEncoder().encode(input) : input;
  let hash = 0xcbf29ce484222325n;
  for (const byte of bytes) {
    hash ^= BigInt(byte);
    hash = (hash * 0x100000001b3n) & 0xffffffffffffffffn;
  }
  return hash.toString(16).padStart(16, "0");
}

/**
 * Confere que a fonte do shader empacotada no bundle é exatamente a que o
 * contrato descreve (mesmos bytes que o Rust carrega com `include_str!`).
 * Se alguém editar um `.wgsl`/`.glsl`, o viewport falha na inicialização em vez
 * de renderizar diferente do headless.
 */
export function assertShaderSource(name: string, source: string): ShaderSourceV1 {
  const shader = shaderOf(name);
  const actual = fnv1a64(source);
  if (actual !== shader.fnv1a64) {
    throw new RenderContractError(
      "shader_drift",
      `shader '${name}' divergiu do contrato (esperado ${shader.fnv1a64}, encontrado ${actual})`
    );
  }
  return shader;
}

/** Impressão digital do toon ramp gerado neste runtime. */
export function toonRampFingerprint(bytes: Uint8Array = toonRampBytes()): string {
  return fnv1a64(bytes);
}

/** Impressão digital congelada do toon ramp (a que o Rust também precisa reproduzir). */
export function expectedToonRampFingerprint(): string {
  return RENDER_CONTRACT.toon_ramp.bytes_fnv1a64;
}

/** Bytes da textura de toon ramp (256×4 RGBA) descrita no contrato. */
export function toonRampBytes(): Uint8Array {
  const spec = RENDER_CONTRACT.toon_ramp;
  if (spec.format !== "rgba8unorm") {
    throw new RenderContractError("unknown_ramp_format", `toon ramp em '${spec.format}' não é suportado`);
  }
  const data = new Uint8Array(spec.width * spec.height * 4);
  for (let row = 0; row < spec.height; row++) {
    const definition = spec.rows[row];
    if (!definition) throw new RenderContractError("missing_ramp_row", `toon ramp sem a linha ${row}`);
    for (let x = 0; x < spec.width; x++) {
      const u = x / (spec.width - 1);
      let factor = u;
      if (definition.kind === "steps") {
        factor = definition.base ?? 0;
        for (const step of definition.steps ?? []) {
          if (u >= step.threshold) factor = step.value;
        }
      }
      const value = Math.min(255, Math.max(0, Math.round(factor * 255)));
      const index = (row * spec.width + x) * 4;
      data[index] = value;
      data[index + 1] = value;
      data[index + 2] = value;
      data[index + 3] = 255;
    }
  }
  return data;
}
