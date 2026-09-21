/**
 * ANIGO render contract v1 — tests de paridade headless ↔ viewport
 * (P0 "Consolidar o renderer", item 7).
 *
 * Não há GPU no ambiente de teste, então a paridade é verificada nos pontos que
 * *definem* a imagem e são testáveis fora da GPU:
 *   * os bytes de cada shader (o que o Rust compila com `include_str!` é o que o
 *     viewport empacota com `?raw`);
 *   * o layout de vértice, dos uniforms e os offsets de cada campo;
 *   * o grafo de passes com cull/blend/depth/bias/MSAA/formatos;
 *   * a câmera e o toon ramp — números congelados que os dois lados reproduzem.
 *
 * Um render dourado de verdade exige GPU: `baselines/` fica documentado como o
 * lugar onde ele entra quando o ambiente tiver adapter (`RELATORIO`/CI de GPU).
 */
import test from "node:test";
import assert from "node:assert/strict";

import {
  RENDER_CONTRACT,
  RenderContractError,
  assertShaderSource,
  addressMode,
  blendStateOf,
  computePasses,
  depthFormat,
  expectedToonRampFingerprint,
  filterMode,
  fnv1a64,
  msaaSampleCount,
  offscreenColorFormat,
  readRenderContract,
  renderPassOrder,
  renderPasses,
  shaderOf,
  toonRampBytes,
  toonRampFingerprint,
  uniformFloats,
  uniformOffset,
  uniformSize,
  vertexBufferLayout,
} from "../../src/contracts/render_contract.v1.ts";
import { RENDER_CONTRACT_DATA } from "../../src/contracts/render_contract_data.v1.ts";
import {
  modelMatrixFromTransform,
  normalMatrixFromModel,
  perspectiveRhZeroToOne,
  viewProjectionMatrix,
} from "../../src/services/camera_math.ts";
import {
  cameraUniformFloats,
  lightUniformFloats,
  materialUniformFloats,
  outlineUniformFloats,
  sparseMorphHeader,
} from "../../src/services/render_uniforms.ts";
import { readRepoFile } from "./rust_contract_source.ts";

const FIXTURE_PATH = "contracts/fixtures/render_contract_v1.json";
const TOLERANCE = 1e-5;

function fixture(): any {
  return JSON.parse(readRepoFile(FIXTURE_PATH));
}

function findShader(name: string) {
  const shader = RENDER_CONTRACT.shaders.find((candidate) => candidate.name === name);
  assert.ok(shader, `shader '${name}' não está no contrato`);
  return shader;
}

function closeTo(actual: ArrayLike<number>, expected: readonly number[], label: string): void {
  assert.equal(actual.length, expected.length, `${label}: tamanho diferente`);
  for (let index = 0; index < expected.length; index += 1) {
    assert.ok(
      Math.abs(actual[index] - expected[index]) < TOLERANCE,
      `${label}[${index}]: ${actual[index]} != ${expected[index]}`
    );
  }
}

// ---------------------------------------------------------------------------
// Fonte única
// ---------------------------------------------------------------------------

test("o fixture congelado e o módulo gerado são o mesmo documento", () => {
  assert.deepEqual(fixture(), JSON.parse(JSON.stringify(RENDER_CONTRACT_DATA)));
  assert.equal(RENDER_CONTRACT.version, 1);
});

test("cada shader do contrato existe no caminho declarado com o hash congelado", () => {
  assert.ok(RENDER_CONTRACT.shaders.length >= 8, "o contrato precisa listar WGSL + fallback GLSL");
  const hashes = new Map<string, string>();
  for (const shader of RENDER_CONTRACT.shaders) {
    const source = readRepoFile(shader.path);
    assert.equal(fnv1a64(source), shader.fnv1a64, `${shader.name}: bytes divergiram do contrato`);
    assert.equal(
      source.split("\n").length - (source.endsWith("\n") ? 1 : 0),
      shader.lines,
      `${shader.name}: contagem de linhas divergiu do contrato`
    );
    assert.equal(shader.path.endsWith(`.${shader.language}`), true, `${shader.name}: extensão vs linguagem`);
    hashes.set(shader.name, shader.fnv1a64);
  }
  // nenhum shader de produção fora de crates/anigo-renderer/shaders/
  for (const shader of RENDER_CONTRACT.shaders) {
    if (shader.role === "fallback_webgl2") {
      assert.match(shader.path, /^crates\/anigo-renderer\/shaders\/webgl2_fallback\//);
    } else if (shader.role !== "library") {
      assert.match(shader.path, /^crates\/anigo-renderer\/shaders\//);
    }
  }
  assert.equal(new Set(hashes.values()).size, hashes.size, "hashes de shader duplicados");
});

test("o repositório tem uma única cópia de cada shader de produção", () => {
  // Rust: os shaders entram por constantes do contrato, nunca por `include_str!` solto
  const headless = readRepoFile("crates/anigo-renderer/src/headless.rs");
  assert.equal(headless.includes('include_str!("../shaders/'), false, "headless.rs voltou a carregar shader direto");
  assert.match(headless, /contract::CEL_SHADING_WGSL/);
  assert.match(headless, /contract::INVERTED_HULL_WGSL/);
  assert.match(headless, /contract::MORPH_SPARSE_COMPUTE_WGSL/);

  const renderContract = readRepoFile("crates/anigo-renderer/src/render_contract.rs");
  for (const shader of RENDER_CONTRACT.shaders) {
    if (shader.role !== "production") continue;
    const file = shader.path.split("/").pop();
    const quoted = renderContract.match(new RegExp(`include_str!\\("\\../shaders/${file}"\\)`, "g")) ?? [];
    assert.equal(quoted.length, 1, `${file}: esperado exatamente um include_str! canônico`);
  }

  // Viewport: importa com `?raw` do caminho canônico, uma vez cada.
  // Shaders `library` (ex.: face_sdf) ainda não têm consumidor — entram no
  // contrato para não virarem uma segunda cópia quando forem usados.
  const renderer = readRepoFile("src/components/viewport/webgpu_renderer.ts");
  for (const shader of RENDER_CONTRACT.shaders) {
    if (shader.role === "library") continue;
    const occurrences = renderer.split(`"../../../${shader.path}?raw"`).length - 1;
    assert.equal(occurrences, 1, `${shader.name}: import ?raw do caminho canônico ausente/duplicado`);
  }
  assert.match(renderer, /assertShaderSource\("cel_shading"/);
  assert.match(renderer, /assertShaderSource\("webgl2_fallback\/cel_fragment"/);
});

// ---------------------------------------------------------------------------
// Layouts: vértice, uniforms, bind groups
// ---------------------------------------------------------------------------

test("vertex layout de 72 B fecha com o bloco vertex_raw", () => {
  const layout = vertexBufferLayout();
  assert.equal(layout.arrayStride, 72);
  assert.equal(uniformSize("vertex_raw"), 72);
  assert.deepEqual(
    layout.attributes.map((attribute) => [attribute.shaderLocation, attribute.offset, attribute.format]),
    [
      [0, 0, "float32x3"],
      [1, 12, "float32x3"],
      [2, 24, "float32x2"],
      [3, 32, "float32x4"],
      [4, 48, "uint16x4"],
      [5, 56, "float32x4"],
    ]
  );
  const raw = RENDER_CONTRACT.uniforms.vertex_raw;
  assert.deepEqual(
    raw.fields.map((field) => [field.name, field.offset, field.size]),
    [
      ["pos", 0, 12],
      ["norm", 12, 12],
      ["uv", 24, 8],
      ["color", 32, 16],
      ["joints", 48, 16],
      ["weights", 56, 16],
    ]
  );
});

test("uniformes: tamanho em floats, offsets e espaços de memória", () => {
  assert.equal(uniformSize("camera"), 208);
  assert.equal(uniformFloats("camera"), 52);
  assert.equal(uniformFloats("light"), 20);
  assert.equal(uniformFloats("material"), 28);
  assert.equal(uniformFloats("outline"), 12);
  assert.equal(uniformFloats("sparse_morph_header"), 4);
  // P1-04: paleta de skinning — 24 ossos × mat4 (16 floats) = 384 floats
  assert.equal(uniformSize("bones"), 1536);
  assert.equal(uniformFloats("bones"), 384);
  assert.equal(uniformOffset("bones", "matrices"), 0);
  assert.equal(RENDER_CONTRACT.uniforms.bones.struct, "BonePalette");

  assert.equal(uniformOffset("camera", "view_proj"), 0);
  assert.equal(uniformOffset("camera", "camera_pos"), 64);
  assert.equal(uniformOffset("camera", "model"), 80);
  assert.equal(uniformOffset("camera", "normal_mat"), 144);
  assert.equal(uniformOffset("material", "params"), 64);
  assert.equal(uniformOffset("material", "params2"), 80);
  assert.equal(uniformOffset("material", "params3"), 96);
  assert.equal(uniformOffset("light", "ambient_ground"), 64);
  assert.equal(uniformOffset("outline", "params2"), 32);

  const spaces = Object.fromEntries(
    Object.entries(RENDER_CONTRACT.uniforms).map(([name, layout]) => [name, layout.address_space])
  );
  assert.deepEqual(spaces, {
    bones: "uniform",
    camera: "uniform",
    light: "uniform",
    material: "uniform",
    outline: "uniform",
    sparse_morph_header: "uniform",
    morph_channel: "storage_read",
    sparse_morph_delta: "storage_read",
    vertex_raw: "vertex_and_storage_read",
  });
  // `vertex_raw` tem 72 B (não múltiplo de 16) porque o WGSL o declara escalar
  // por escalar — se virar array de vec3 o stride passaria a exigir 80.
  assert.match(String(RENDER_CONTRACT.uniforms.vertex_raw.note), /scalar/);
});

test("os blocos de uniform batem com os structs #[repr(C)] do Rust", () => {
  const rust = readRepoFile("crates/anigo-renderer/src/uniforms.rs");
  const expected: Record<string, string[]> = {
    camera: ["view_proj", "camera_pos", "model", "normal_mat"],
    light: ["direction", "color", "shadow_color", "ambient_sky", "ambient_ground"],
    material: ["base_color", "shade_color", "specular_color", "rim_color", "params", "params2", "params3"],
    outline: ["color", "params", "params2"],
  };
  for (const [block, fields] of Object.entries(expected)) {
    const structName = `pub struct ${block[0].toUpperCase()}${block.slice(1)}Uniform {`;
    const start = rust.indexOf(structName);
    assert.ok(start >= 0, `struct ${structName} não encontrado em uniforms.rs`);
    const body = rust.slice(start, rust.indexOf("}", start));
    const declared = Array.from(body.matchAll(/pub (\w+): \[f32; (\d+)\]/g)).map((match) => match[1]);
    assert.deepEqual(declared, fields, `${block}: campos divergem do contrato`);
    const bytes = Array.from(body.matchAll(/\[f32; (\d+)\]/g)).reduce(
      (total, match) => total + Number(match[1]) * 4,
      0
    );
    assert.equal(bytes, uniformSize(block), `${block}: bytes divergem do contrato`);
    assert.equal(new Set(fields).size, fields.length);
    assert.deepEqual(
      RENDER_CONTRACT.uniforms[block].fields.map((field) => field.name),
      fields,
      `${block}: ordem dos campos divergiu do contrato`
    );
  }
  // o bloco de câmera precisa ser o declarado no contrato
  assert.equal(RENDER_CONTRACT.camera.uniform, "camera");
  assert.match(rust, /pub normal_mat: \[f32; 16\]/);
});

test("bind groups conferem com as declarações do WGSL", () => {
  for (const group of RENDER_CONTRACT.bind_groups) {
    assert.equal(group.group, 0);
    assert.ok(group.entries.length > 0, `${group.name}: bind group vazio`);
    for (const entry of group.entries) {
      assert.ok(entry.declaration.trim().length > 0, `${group.name}:${entry.binding} sem declaração`);
      assert.ok(entry.stages.length > 0, `${group.name}:${entry.binding} sem estágios`);
    }
  }
  for (const pass of [...renderPasses(), ...computePasses()]) {
    const shader = findShader(pass.shader);
    assert.ok(
      RENDER_CONTRACT.bind_groups.some((group) => group.name === pass.bind_group),
      `${pass.name}: bind group '${pass.bind_group}' não existe`
    );
    assert.ok(shader, `${pass.name}: shader '${pass.shader}' não existe`);
    const group = RENDER_CONTRACT.bind_groups.find((candidate) => candidate.name === pass.bind_group)!;
    assert.deepEqual(
      group.entries.map((entry) => entry.binding),
      group.entries.map((_, index) => index),
      `${pass.name}: bindings precisam ser 0..n-1`
    );
  }
});

// ---------------------------------------------------------------------------
// Passes / pipeline state
// ---------------------------------------------------------------------------

test("o grafo de passes é o mesmo nos dois renderers", () => {
  assert.deepEqual(renderPassOrder(), ["outline", "cel"]);
  const outline = RENDER_CONTRACT.passes.find((pass) => pass.name === "outline")!;
  assert.equal(outline.cull_mode, "front");
  assert.equal(outline.depth_write, false);
  assert.deepEqual(outline.depth_bias, { constant: 1, slope_scale: 1, clamp: 0 });
  assert.equal(outline.blend, "src_alpha_one_minus_src_alpha");
  assert.deepEqual(blendStateOf(outline), {
    color: { srcFactor: "src-alpha", dstFactor: "one-minus-src-alpha", operation: "add" },
    alpha: { srcFactor: "one", dstFactor: "one-minus-src-alpha", operation: "add" },
  });

  const cel = RENDER_CONTRACT.passes.find((pass) => pass.name === "cel")!;
  assert.equal(cel.cull_mode, "back");
  assert.equal(cel.depth_write, true);
  assert.equal(cel.blend, "none");
  assert.equal(blendStateOf(cel), undefined);
  assert.equal(cel.depth_compare, "less-equal");

  assert.deepEqual(
    computePasses().map((pass) => [pass.name, pass.shader, pass.workgroup_size, pass.only_when]),
    [["sparse_morph", "morph_sparse_compute", 64, "gpu_morph_active"]]
  );
  assert.ok(outline.order < cel.order, "outline precisa ser desenhado antes do cel");
  assert.ok(computePasses()[0].order < outline.order, "o compute de morphs vem antes do render");
});

test("alvos, MSAA e formatos vêm do contrato nos dois lados", () => {
  assert.equal(offscreenColorFormat(), "rgba8unorm");
  assert.equal(depthFormat(), "depth24plus");
  assert.equal(msaaSampleCount(), 4);
  assert.equal(RENDER_CONTRACT.targets.resolve_to_swapchain, true);
  assert.equal(RENDER_CONTRACT.targets.viewport_color_format_policy, "surface_preferred");
  assert.equal(RENDER_CONTRACT.targets.clear.source, "scene.background_color");

  const renderer = readRepoFile("src/components/viewport/webgpu_renderer.ts");
  assert.match(renderer, /private sampleCount: number = msaaSampleCount\(\)/);
  assert.match(renderer, /multisample: \{ count: msaaSampleCount\(\) \}/);
  assert.match(renderer, /size: uniformSize\("camera"\)/);
  assert.match(renderer, /const contractVertexLayout = vertexBufferLayout\(\)/);
  assert.match(renderer, /for \(const pass of renderPasses\(\)\)/);
  // literais de layout que saíram do renderer (se voltarem, a unificação se perde)
  for (const forbidden of ['size: 208', 'size: 112', 'size: 48,', 'sampleCount: 4', 'format: "depth24plus"', 'arrayStride: 72']) {
    assert.equal(renderer.includes(forbidden), false, `renderer ainda tem o literal ${forbidden}`);
  }

  const headless = readRepoFile("crates/anigo-renderer/src/headless.rs");
  assert.match(headless, /contract::offscreen_color_format\(\)/);
  assert.match(headless, /contract::depth_format\(\)/);
  assert.match(headless, /contract::msaa_sample_count\(\)/);
  assert.match(headless, /resolve_target,/);
  assert.match(headless, /for pass_name in contract::render_pass_order\(\)/);
  assert.equal(/sample_count: 1,\n\s+dimension: wgpu::TextureDimension::D2,\n\s+format: wgpu::TextureFormat::Rgba8Unorm,/.test(headless), false);
  const toonRamp = readRepoFile("crates/anigo-renderer/src/render_contract.rs");
  assert.match(toonRamp, /pub fn toon_ramp_bytes\(\)/);
  assert.match(toonRamp, /expected_toon_ramp_fingerprint/);
});

// ---------------------------------------------------------------------------
// Câmera / transforms
// ---------------------------------------------------------------------------

test("as convenções de câmera do contrato são as implementadas", () => {
  assert.equal(RENDER_CONTRACT.camera.projection, "perspective_rh");
  assert.equal(RENDER_CONTRACT.camera.clip_depth, "zero_to_one");
  assert.equal(RENDER_CONTRACT.camera.matrix_layout, "column_major");
  assert.equal(RENDER_CONTRACT.camera.up_axis, "y");
  assert.equal(RENDER_CONTRACT.camera.model_from, "scene.nodes[0].transform");

  // profundidade 0..1: near → 0, far → 1 (clip space do wgpu)
  const proj = perspectiveRhZeroToOne(45 * (Math.PI / 180), 16 / 9, 0.05, 100);
  const project = (z: number) => proj[10] * z + proj[14];
  const divide = (z: number) => project(z) / (proj[11] * z);
  assert.ok(Math.abs(divide(-0.05) - 0) < 1e-6, `near deveria mapear para 0, deu ${divide(-0.05)}`);
  assert.ok(Math.abs(divide(-100) - 1) < 1e-6, `far deveria mapear para 1, deu ${divide(-100)}`);
});

test("o frame congelado do contrato é reproduzido pelos packers do viewport", () => {
  const reference = fixture().reference_frame;
  const camera = reference.camera;

  const viewProj = viewProjectionMatrix(
    { eye: camera.eye, target: camera.target, up: camera.up, fov: camera.fov_y_radians },
    camera.aspect,
    { near: camera.z_near, far: camera.z_far }
  );
  closeTo(viewProj, reference.expected.view_proj, "view_proj");

  const model = modelMatrixFromTransform(reference.model);
  closeTo(model, reference.expected.model_matrix, "model_matrix");

  const normal = normalMatrixFromModel(model);
  closeTo(normal, reference.expected.normal_matrix, "normal_matrix");
  // o modelo tem rotação: a normal matrix precisa ter termos cruzados e a
  // translação zerada (pega transposição/inversa erradas e padding de vec3)
  assert.notEqual(normal[0], normal[8]);
  assert.equal(normal[12], 0);
  assert.equal(normal[13], 0);
  assert.equal(normal[14], 0);
  assert.equal(normal[15], 1);

  const cameraUniform = cameraUniformFloats({ viewProj, eye: camera.eye, model, normalMat: normal });
  closeTo(cameraUniform, reference.expected.camera_uniform, "camera_uniform");
  closeTo(cameraUniform.slice(20, 36), model, "camera.model");
  closeTo(cameraUniform.slice(36, 52), normal, "camera.normal_mat");

  const light = reference.light;
  closeTo(
    lightUniformFloats({
      direction: light.direction,
      intensity: light.intensity,
      color: light.color,
      ambientIntensity: light.ambient_intensity,
      shadowColor: light.shadow_color,
      shadowSaturation: light.shadow_saturation,
      ambientSky: light.ambient_sky,
      ambientGround: light.ambient_ground,
    }),
    reference.expected.light_uniform,
    "light_uniform"
  );

  const material = reference.material;
  closeTo(
    materialUniformFloats({
      baseColor: material.base_color,
      shadeColor: material.shade_color,
      specularColor: material.specular_color,
      rimColor: material.rim_color,
      shadowThreshold: material.shadow_threshold,
      shadowSmoothness: material.shadow_smoothness,
      specIntensity: material.spec_intensity,
      specPower: material.spec_power,
      rimIntensity: material.rim_intensity,
      rimSpread: material.rim_spread,
      hueShiftDegrees: material.hue_shift_degrees,
      toonSteps: material.toon_steps,
      specularSoftness: material.specular_softness,
      specularOffset: material.specular_offset,
      specularSize: material.specular_size,
      aoIntensity: material.ao_intensity,
    }),
    reference.expected.material_uniform,
    "material_uniform"
  );

  closeTo(
    outlineUniformFloats({
      color: material.outline_color,
      width: material.outline_width,
      aspect: camera.aspect,
      depthBias: material.outline_depth_bias,
      opacity: material.outline_opacity,
      smoothness: material.outline_smoothness,
    }),
    reference.expected.outline_uniform,
    "outline_uniform"
  );
});

test("o cabeçalho do compute de morphs preenche só os campos do contrato", () => {
  const header = sparseMorphHeader(3, 4070, 512);
  assert.equal(header.length, 4);
  assert.equal(header[uniformOffset("sparse_morph_header", "active_channel_count") / 4], 3);
  assert.equal(header[uniformOffset("sparse_morph_header", "total_vertex_count") / 4], 4070);
  assert.equal(header[uniformOffset("sparse_morph_header", "total_delta_count") / 4], 512);
  assert.equal(header[3], 0, "o padding precisa ficar zerado");
});

test("o viewport não mantém uma segunda implementação de câmera", () => {
  const renderer = readRepoFile("src/components/viewport/webgpu_renderer.ts");
  assert.match(renderer, /return viewProjectionMatrix\(/, "o viewport precisa usar src/services/camera_math.ts");
  assert.match(renderer, /clipDepth: isWebGPU \? "zero_to_one" : "minus_one_to_one"/);
  // a cópia antiga montava a perspectiva na mão (f / aspect, far * nf, …)
  for (const forbidden of ["far * nf", "const nf = 1.0 / (near - far)", "-this.dot(right, this.eye)"]) {
    assert.equal(renderer.includes(forbidden), false, `câmera duplicada de volta: ${forbidden}`);
  }
  assert.match(readRepoFile("src/services/camera_math.ts"), /export function viewProjectionMatrix\(/);
});

test("a verificação de bytes de shader em runtime está ativa", () => {
  const tampered = readRepoFile("crates/anigo-renderer/shaders/cel_shading.wgsl") + "\n";
  assert.throws(
    () => assertShaderSource("cel_shading", tampered),
    (error: unknown) => {
      assert.ok(error instanceof RenderContractError);
      assert.equal((error as InstanceType<typeof RenderContractError>).code, "shader_drift");
      return true;
    }
  );
  assert.doesNotThrow(() =>
    assertShaderSource("cel_shading", readRepoFile("crates/anigo-renderer/shaders/cel_shading.wgsl"))
  );
});

// ---------------------------------------------------------------------------
// Toon ramp
// ---------------------------------------------------------------------------

test("toon ramp: bytes, linhas e sampler iguais nos dois renderers", () => {
  const bytes = toonRampBytes();
  assert.equal(bytes.length, RENDER_CONTRACT.toon_ramp.bytes_len);
  assert.equal(bytes.length, 4096);
  assert.equal(toonRampFingerprint(bytes), expectedToonRampFingerprint());
  assert.equal(toonRampFingerprint(bytes), "334eb404f19d1a4d");

  const sample = RENDER_CONTRACT.toon_ramp.sample_u8;
  for (let index = 0; index < sample.length; index += 1) {
    assert.equal(bytes[index * 256], sample[index], `ramp byte ${index * 256}`);
  }
  // linhas: 0 contínua, 1 degrau único, 2 penumbra, 3 três degraus
  assert.equal(bytes[0], 0);
  assert.equal(bytes[255 * 4], 255);
  assert.equal(bytes[(256 + 127) * 4], 0);
  assert.equal(bytes[(256 + 128) * 4], 255);
  assert.equal(bytes[(2 * 256 + 64) * 4], 0);
  assert.equal(bytes[(2 * 256 + 128) * 4], 128);
  assert.equal(bytes[(3 * 256 + 64) * 4], 89);
  assert.equal(bytes[(3 * 256 + 128) * 4], 179);

  assert.equal(filterMode(RENDER_CONTRACT.toon_ramp.mag_filter), "linear");
  assert.equal(filterMode(RENDER_CONTRACT.toon_ramp.min_filter), "linear");
  assert.equal(addressMode(RENDER_CONTRACT.toon_ramp.address_mode), "clamp-to-edge");
  const renderer = readRepoFile("src/components/viewport/webgpu_renderer.ts");
  assert.match(renderer, /const rampData = toonRampBytes\(\)/);
  assert.match(renderer, /rampFingerprint !== expectedToonRampFingerprint\(\)/);
});

// ---------------------------------------------------------------------------
// Validação do contrato: esquema inválido dá erro explícito (recuperável)
// ---------------------------------------------------------------------------

test("contrato inválido falha com erro explícito e recuperável", () => {
  const base = fixture();
  const mutate = (change: (copy: any) => void) => {
    const copy = JSON.parse(JSON.stringify(base));
    change(copy);
    return copy;
  };
  const rejects = (copy: any, code: string) => {
    assert.throws(
      () => readRenderContract(copy),
      (error: unknown) => {
        assert.ok(error instanceof RenderContractError, `esperado RenderContractError, veio ${error}`);
        const contractError = error as InstanceType<typeof RenderContractError>;
        assert.equal(contractError.code, code);
        assert.equal(contractError.recoverable, true);
        assert.ok(contractError.message.length > 0);
        return true;
      }
    );
  };

  rejects(mutate((copy) => (copy.version = 2)), "unsupported_version");
  rejects(mutate((copy) => (copy.shaders = [])), "missing_shaders");
  rejects(mutate((copy) => copy.shaders.forEach((shader: any) => (shader.role = "library"))), "missing_shaders");
  rejects(mutate((copy) => (copy.shaders = "nada")), "missing_shaders");
  rejects(null, "not_object");
  rejects(mutate((copy) => (copy.shaders[0].path = "crates/anigo-renderer/shaders/cel_shading.glsl")), "bad_shader_path");
  rejects(mutate((copy) => (copy.shaders[0].fnv1a64 = "xyz")), "bad_shader_hash");
  rejects(mutate((copy) => (copy.passes = [])), "missing_passes");
  rejects(mutate((copy) => (copy.passes[0].shader = "nao_existe")), "unknown_shader");
  rejects(mutate((copy) => (copy.passes[0].bind_group = "nao_existe")), "unknown_bind_group");
  rejects(mutate((copy) => (copy.uniforms.camera.size = 210)), "bad_uniform_size");
  rejects(mutate((copy) => (copy.uniforms.vertex_raw.address_space = "teleport")), "bad_address_space");
  rejects(
    mutate((copy) => (copy.uniforms.outline.fields[2].offset = 44)),
    "bad_uniform_field"
  );
  assert.throws(() => shaderOf("nao_existe"), /não está no contrato/);
  assert.throws(() => uniformOffset("camera", "nao_existe"), /não está no contrato/);
  assert.throws(() => blendStateOf({ name: "x", blend: "mágica" } as any), /desconhecido/);
});

test("o contrato válido passa pela validação e é o mesmo objeto exposto", () => {
  const validated = readRenderContract(fixture());
  assert.equal(validated.version, RENDER_CONTRACT.version);
  assert.deepEqual(
    validated.shaders.map((shader) => shader.name),
    RENDER_CONTRACT.shaders.map((shader) => shader.name)
  );
  assert.equal(RENDER_CONTRACT, RENDER_CONTRACT_DATA, "o contrato exposto precisa ser o dado gerado");
});
