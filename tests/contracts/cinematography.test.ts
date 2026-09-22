/**
 * ANIGO — Câmera Cinematográfica (Fase 2 #53): golden tests + paridade de contrato.
 *
 * Critérios de aceite do issue:
 *   1. DoF desfoca o fundo enquanto o rosto/olhos permanecem nítidos
 *      (CoC = 0 no plano de foco; amostra central sempre incluída).
 *   2. Alternar de lente (24 → 85 mm) muda a perspectiva SEM mover o
 *      orbitador bruscamente (reframe = r × tan(fov_from/2)/tan(fov_to/2)).
 *   3. Tracking de alvo (cabeça/Hips/POI) mantém o enquadramento com
 *      amortecimento exponencial (translação rígida do rig target+eye).
 *
 * Os números dourados são independentes (precisão dupla → f32) e congelados
 * em `render_contract.cinematography`; a referência TS
 * (src/services/camera_cinematic.ts) precisa reproduzi-los. O Rust
 * (anigo-core::math) e o shader (postprocess_dof.wgsl) implementam as
 * MESMAS fórmulas — paridade verificada por inspeção de fonte (cargo não
 * existe neste sandbox; `cargo test` roda na CI).
 */
import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import {
  SENSOR_HEIGHT_MM,
  LENS_PRESETS,
  lensFovY,
  reframeRadius,
  circleOfConfusion,
  bokehRadiusPx,
  linearizeDepth,
  dampValue,
  dampVec3,
  TRACKING_DAMPING_DEFAULT,
  TRACKING_TARGET_POSITIONS,
  trackingDesiredTarget,
  stepTracking,
  DEFAULT_DOF_SETTINGS,
} from "../../src/services/camera_cinematic";
import {
  RENDER_CONTRACT,
  uniformSize,
  uniformOffset,
  uniformFloats,
} from "../../src/contracts/render_contract.v1";
import { dofUniformFloats } from "../../src/services/render_uniforms";
import { defaultSceneDomain, parseSceneDomain } from "../../src/services/project_persistence";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "..");
const readRepoFile = (relative) => fs.readFileSync(path.join(root, relative), "utf8");

const approx = (a: number, b: number, tol: number, msg?: string) =>
  assert.ok(Math.abs(a - b) <= tol, `${msg ?? "valor"}: ${a} ≠ ${b} (tol ${tol})`);

test("lentes: presets do contrato batem com lensFovY (full-frame 24 mm)", () => {
  const cine = RENDER_CONTRACT.cinematography;
  assert.ok(cine, "seção cinematography ausente do render contract");
  assert.equal(cine.lens.sensor_height_mm, SENSOR_HEIGHT_MM);

  assert.equal(LENS_PRESETS.length, cine.lens.presets.length);
  for (let i = 0; i < LENS_PRESETS.length; i++) {
    const preset = LENS_PRESETS[i];
    const golden = cine.lens.presets[i];
    assert.equal(preset.id, golden.id);
    assert.equal(preset.focalMm, golden.focal_mm);
    assert.equal(preset.name, golden.name);
    const fovDeg = (lensFovY(preset.focalMm) * 180) / Math.PI;
    approx(fovDeg, golden.fov_y_degrees, 1e-4, `fov ${preset.id} mm`);
  }
  // Monotônico: focal mais longa → FOV menor.
  for (let i = 1; i < LENS_PRESETS.length; i++) {
    assert.ok(lensFovY(LENS_PRESETS[i].focalMm) < lensFovY(LENS_PRESETS[i - 1].focalMm));
  }
});

test("lentes: reframe conserva o enquadramento sem salta (aceite 2)", () => {
  const golden = RENDER_CONTRACT.cinematography!.lens.reframe.golden;
  const from = lensFovY(golden.from_mm);
  const to = lensFovY(golden.to_mm);
  const expected = reframeRadius(golden.radius, from, to);
  approx(expected, golden.expected_radius, 1e-3, "raio do reframe 50→85 mm");
  // Enquadramento conservado: tan(fov/2) × raio invariante.
  const framing = (fov: number, radius: number) => Math.tan(fov / 2) * radius;
  approx(framing(from, golden.radius), framing(to, expected), 1e-9, "framing invariante");
  // Telefoto afasta o orbitador; grande angular aproxima.
  assert.ok(expected > golden.radius, "85 mm precisa afastar o orbitador");
  assert.ok(reframeRadius(golden.radius, to, from) < golden.radius, "24 mm precisa aproximar");
});

test("DoF: golden do CoC (fórmula do issue) + in focus = 0 (aceite 1)", () => {
  const golden = RENDER_CONTRACT.cinematography!.dof.golden;
  assert.ok(golden.length >= 5, "contrato precisa dos 5 goldens de CoC");
  for (const row of golden) {
    const coc = circleOfConfusion(row.frag_dist, row.focus_dist, row.focal_m, row.f_number);
    if (row.expected_coc === 0) {
      assert.ok(coc < 1e-6, `${row.name}: CoC no plano de foco precisa ser ~0 (got ${coc})`);
    } else {
      approx(coc, row.expected_coc, Math.max(1e-8, row.expected_coc * 1e-3), `CoC ${row.name}`);
    }
  }
  // Física: mais aberto (N menor) e mais tele → mais bokeh.
  assert.ok(circleOfConfusion(1.0, 2.0, 0.05, 1.4) > circleOfConfusion(1.0, 2.0, 0.05, 2.0));
  assert.ok(
    circleOfConfusion(1.0, 2.0, 0.085, 2.0) > circleOfConfusion(1.0, 2.0, 0.05, 2.0)
  );
});

test("DoF: raio de bokeh em px — conversão sensor→pixels + teto", () => {
  const golden = RENDER_CONTRACT.cinematography!.dof.bokeh_px_golden;
  for (const row of golden.values) {
    const maxR = row.max_radius_px ?? Infinity;
    if (row.coc === undefined) {
      // Linha de clamp sem CoC: só verifica o teto rígido.
      assert.ok(maxR < Infinity, `linha ${row.name} precisa de max_radius_px`);
      continue;
    }
    const radius = bokehRadiusPx(row.coc, golden.image_height_px, golden.sensor_height_m, maxR);
    approx(radius, row.expected_radius_px, 0.01, `bokeh ${row.name}`);
  }
  // O teto de custo sempre domina.
  assert.ok(
    bokehRadiusPx(1.0, 1080, 0.024, 16.0) === 16.0,
    "max_radius_px precisa ser um teto rígido"
  );
  // Linearização da profundidade (zero_to_one RH) bate com o shader:
  // z = near·far / (far − d·(far − near)); d=0.5, near=0.05, far=100
  // → 5/50.025 = 0.09995002 (golden).
  approx(linearizeDepth(0.5, 0.05, 100.0), 0.09995002, 1e-6, "d=0.5");
  // zero_to_one: d=0 é o plano próximo (distância mínima), d=1 o far.
  approx(linearizeDepth(0.0, 0.05, 100.0), 0.05, 1e-6, "d=0 → z=near");
  approx(linearizeDepth(1.0, 0.05, 100.0), 100.0, 1e-6, "d=1 → z=far");
});

test("tracking: damping exponencial — golden + convergência sem overshoot", () => {
  const golden = RENDER_CONTRACT.cinematography!.tracking.damping.golden;
  approx(golden.damping_per_second, TRACKING_DAMPING_DEFAULT, 1e-9, "damping padrão");
  let v = dampVec3(golden.from, golden.to, golden.damping_per_second, 0);
  assert.deepEqual(v, golden.from, "dt=0 não move");
  for (let i = 0; i < golden.frames_60fps; i++) {
    v = dampVec3(v, golden.to, golden.damping_per_second, 1.0 / 60.0);
  }
  for (let c = 0; c < 3; c++) {
    approx(v[c], golden.expected[c], 1e-6, `damping componente ${c}`);
  }
  // Nunca ultrapassa o alvo (amortecimento crítico).
  const x = dampValue(0.0, 1.0, 6.0, 1.0 / 60.0);
  assert.ok(x < 1.0, "damping não pode ultrapassar");
  assert.equal(dampValue(0.5, 1.0, 0.0, 1.0), 0.5, "damping 0 → imutável");
});

test("tracking: stepTracking — rig rígido, modo off inerte (aceite 3)", () => {
  // off: nada se move.
  const off = stepTracking([0, 1, 3.5], [0, 1.5, 5.5], "off", 6.0, [0, 0, 0], 1 / 60);
  assert.equal(off.moved, false);
  assert.deepEqual(off.target, [0, 1, 3.5]);

  // head: o alvo persegue a cabeça e o olho move com o MESMO delta
  // (distância olho→alvo constante = enquadramento mantido).
  const t0: [number, number, number] = [0.0, 1.0, 0.0];
  const e0: [number, number, number] = [0.0, 1.5, 3.5];
  const dist0 = Math.hypot(e0[0] - t0[0], e0[1] - t0[1], e0[2] - t0[2]);
  let t = t0;
  let e = e0;
  for (let i = 0; i < 600; i++) {
    const s = stepTracking(t, e, "head", 6.0, [0, 0, 0], 1 / 60);
    t = s.target;
    e = s.eye;
  }
  approx(t[0], TRACKING_TARGET_POSITIONS.head[0], 1e-3, "target.x → cabeça");
  approx(t[1], TRACKING_TARGET_POSITIONS.head[1], 1e-3, "target.y → cabeça");
  approx(t[2], TRACKING_TARGET_POSITIONS.head[2], 1e-3, "target.z → cabeça");
  const distFinal = Math.hypot(e[0] - t[0], e[1] - t[1], e[2] - t[2]);
  approx(distFinal, dist0, 1e-6, "translação rígida: |eye−target| constante");

  // hips e poi: destinos corretos.
  assert.deepEqual(trackingDesiredTarget("hips", [9, 9, 9]), TRACKING_TARGET_POSITIONS.hips);
  assert.deepEqual(trackingDesiredTarget("poi", [1, 2, 3]), [1, 2, 3]);
});

test("contrato: bloco DofUniform (48 B), bind group dof e passe dof_post", () => {
  assert.equal(uniformSize("dof"), 48, "DofUniform = 48 B (12 floats)");
  assert.equal(uniformOffset("dof", "params"), 0);
  assert.equal(uniformOffset("dof", "resolution"), 16);
  assert.equal(uniformOffset("dof", "limits"), 32);
  assert.equal(uniformFloats("dof"), 12);

  const group = RENDER_CONTRACT.bind_groups.find((g) => g.name === "dof");
  assert.ok(group, "bind group 'dof' ausente");
  assert.equal(group.entries.length, 4);
  const byBinding = new Map(group.entries.map((e) => [e.binding, e.kind]));
  assert.equal(byBinding.get(0), "uniform");
  assert.equal(byBinding.get(1), "texture_2d<f32>");
  assert.equal(byBinding.get(2), "texture_2d<f32>");
  assert.equal(byBinding.get(3), "sampler");

  const pass = RENDER_CONTRACT.passes.find((p) => p.name === "dof_post");
  assert.ok(pass, "passe dof_post ausente");
  assert.equal(pass.kind, "render");
  assert.equal(pass.order, 2, "DoF roda DEPOIS de outline(0) e cel(1)");
  assert.equal(pass.shader, "postprocess_dof");
  assert.equal(pass.vertex_entry, "vs_dof");
  assert.equal(pass.fragment_entry, "fs_dof");
  assert.equal(pass.only_when, "dof_enabled");
  assert.equal(pass.fullscreen_triangle, true);

  const shader = RENDER_CONTRACT.shaders.find((s) => s.name === "postprocess_dof");
  assert.ok(shader, "shader postprocess_dof ausente");
  assert.equal(shader.role, "production");
  assert.equal(shader.entry_points.vertex, "vs_dof");
  assert.equal(shader.entry_points.fragment, "fs_dof");
});

test("shader postprocess_dof.wgsl: CoC + bokeh + profundidade (mesma matemática)", () => {
  const src = readRepoFile("crates/anigo-renderer/shaders/postprocess_dof.wgsl");
  // A MESMA fórmula do contrato/Rust/TS (paridade por construção).
  assert.ok(src.includes("abs(fd - d) / d"), "fórmula do CoC ausente no shader");
  assert.ok(src.includes("near * far") && src.includes("far - d * (far - near)"), "linearização ausente");
  assert.ok(src.includes("0.8660254"), "apótema do hexágono ausente");
  assert.ok(src.includes("array<vec2<f32>, 16>"), "disco de 16 amostras ausente");
  assert.ok(src.includes("radius_px < 0.5"), "curto-circuito em foco ausente");
  // Uniform exatamente como o contrato declara.
  assert.ok(src.includes("struct DofUniform"), "struct DofUniform ausente");
  for (const field of ["params", "resolution", "limits"]) {
    assert.ok(src.includes(`${field}: vec4<f32>`), `campo ${field} ausente`);
  }
});

test("paridade Rust: math.rs, DofSettings, DofUniform, Tauri e headless", () => {
  const math = readRepoFile("crates/anigo-core/src/math.rs");
  for (const fn of [
    "pub fn lens_fov_y",
    "pub fn reframe_radius",
    "pub fn circle_of_confusion",
    "pub fn bokeh_radius_px",
    "pub fn damp_value",
    "pub fn damp_vec3",
    "pub const SENSOR_HEIGHT_MM",
  ]) {
    assert.ok(math.includes(fn), `math.rs sem '${fn}'`);
  }
  const scene = readRepoFile("crates/anigo-core/src/scene.rs");
  assert.ok(scene.includes("pub struct DofSettings"), "scene.rs sem DofSettings");
  assert.ok(scene.includes("pub dof: DofSettings"), "Scene sem campo dof");
  assert.ok(scene.includes('#[serde(default)]'), "dof precisa de serde default (arquivos antigos)");

  const uniforms = readRepoFile("crates/anigo-renderer/src/uniforms.rs");
  assert.ok(uniforms.includes("pub struct DofUniform"), "uniforms.rs sem DofUniform");
  assert.ok(uniforms.includes("from_settings"), "DofUniform::from_settings ausente");

  const headless = readRepoFile("crates/anigo-renderer/src/headless.rs");
  assert.ok(headless.includes("scene.dof.enabled"), "headless não consulta Scene.dof");
  assert.ok(headless.includes("dof_pipeline"), "headless sem pipeline de DoF");
  // O wgpu deste projeto não expõe depth resolve no attachment: a resolve
  // MSAA→1× é manual (copy_texture_to_texture, mesma semântica de min do
  // resolve de cor). O browser usa o depthResolveAttachment da spec.
  assert.ok(headless.includes("encoder.copy_texture_to_texture"), "profundidade do DoF sem resolve 1×");

  const main = readRepoFile("src-tauri/src/main.rs");
  assert.ok(main.includes("async fn set_dof_settings"), "Tauri sem set_dof_settings");
});

test("paridade viewport: pipeline, uniform e tracking por frame", () => {
  const src = readRepoFile("src/components/viewport/webgpu_renderer.ts");
  assert.ok(src.includes('assertShaderSource("postprocess_dof", dofShaderSource)'), "viewport não verifica o hash do shader DoF");
  assert.ok(src.includes("dofUniformFloats"), "viewport não empacota DofUniform");
  assert.ok(src.includes("stepTracking"), "tracking não roda por frame no viewport");
  assert.ok(src.includes("setDofSettings"), "viewport sem setDofSettings");
  assert.ok(src.includes("setTrackingSettings"), "viewport sem setTrackingSettings");
  assert.ok(src.includes("depthResolveAttachment: dofActive"), "resolve de profundidade para o DoF ausente");

  // O empacotador TS bate com o layout do contrato (mesmos bytes do Rust).
  const packed = dofUniformFloats({
    focusDistance: 2.0,
    fNumber: 2.0,
    bokehShape: 1,
    focalMm: 85.0,
    maxRadiusPx: 16.0,
    widthPx: 1280,
    heightPx: 720,
    zNear: 0.05,
    zFar: 100.0,
  });
  assert.equal(packed.length, 12);
  approx(packed[0], 2.0, 1e-9, "focus @0");
  approx(packed[1], 2.0, 1e-9, "f_number @1");
  approx(packed[2], 1.0, 1e-9, "bokeh_shape @2");
  approx(packed[3], 0.085, 1e-9, "focal_m @3");
  approx(packed[4], 1280.0, 1e-9, "width @16");
  approx(packed[5], 720.0, 1e-9, "height @17");
  approx(packed[6], 0.05, 1e-9, "z_near @18");
  approx(packed[7], 100.0, 1e-9, "z_far @19");
  approx(packed[8], 16.0, 1e-9, "max_radius @32");
  approx(packed[9], 0.024, 1e-9, "sensor_height @33");
  approx(packed[10], 0.0, 1e-9, "reserved @34");
  approx(packed[11], 0.0, 1e-9, "reserved @35");
});

test("state: defaults do contrato — DoF e tracking off por padrão", () => {
  assert.equal(DEFAULT_DOF_SETTINGS.enabled, false, "DoF tem que nascer OFF (frame intacto)");
  assert.equal(DEFAULT_DOF_SETTINGS.focusDistance, 2.0);
  assert.equal(DEFAULT_DOF_SETTINGS.fNumber, 2.0);
  assert.equal(DEFAULT_DOF_SETTINGS.focalMm, 50.0);
  assert.equal(DEFAULT_DOF_SETTINGS.bokehShape, 0);
  assert.equal(DEFAULT_DOF_SETTINGS.maxRadiusPx, 16.0);
  assert.equal(TRACKING_DAMPING_DEFAULT, 6.0);
  // A persistência nasce tolerante: sem o bloco dof, default off.
  const domain = parseSceneDomain({});
  assert.equal(domain.dof.enabled, false, "arquivo antigo (sem dof) → DoF off");
  assert.equal(defaultSceneDomain().dof.focal_mm, 50.0);
});
