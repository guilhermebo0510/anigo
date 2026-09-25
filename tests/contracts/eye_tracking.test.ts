/**
 * ANIGO — motor de olhos/íris (Fase 2 #43): golden tests + paridade de contrato.
 *
 * Critérios de aceite do issue:
 *   1. Íris com profundidade convexa ao orbitar a câmera (parallax — fórmula
 *      `UV + V_tangent.xy × depth_scale`, golden no contrato).
 *   2. Highlights visíveis em sombra total (mask desacoplada da iluminação —
 *      golden no contrato + verificação no cel shader).
 *   3. Contato visual inteligente com a câmera (look-at solver com clamps
 *      físicos + micro-sacadas de 2–5° — golden no contrato).
 *
 * Os números dourados são independentes (produzidos fora do pipeline) e
 * congelados em `render_contract.anime_eye`; a referência TS
 * (src/services/eye_tracking.ts) precisa reproduzi-los dentro de 1e-5 —
 * o mesmo mecanismo de paridade do reference frame. O Rust (anigo-ik)
 * espelha as mesmas fórmulas (syntax-check + paridade manual; `cargo test`
 * fica para CI, sem cargo neste sandbox).
 */
import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import {
  EYE_OFFSET_LEFT,
  EYE_OFFSET_RIGHT,
  GAZE_MAX_PITCH_DEGREES,
  GAZE_MAX_YAW_DEGREES,
  GAZE_SACCADE_AMPLITUDE_DEFAULT,
  solveGaze,
  saccades,
  frameGaze,
  dampGaze,
  gazeAngleDegrees,
  eyeParallaxUv,
  eyeHighlightMask,
  eyeHighlightRgb,
} from "../../src/services/eye_tracking";
import { RENDER_CONTRACT } from "../../src/contracts/render_contract.v1";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "..");
const readRepoFile = (relative) => fs.readFileSync(path.join(root, relative), "utf8");

const animeEye = RENDER_CONTRACT.anime_eye;
assert.ok(animeEye, "contrato precisa declarar a seção anime_eye");

function closeTo(actual, expected, label, tolerance = 1e-5) {
  assert.ok(
    typeof actual === "number" && Number.isFinite(actual) && Math.abs(actual - expected) <= tolerance,
    `${label}: esperado ${expected}, veio ${actual}`
  );
}

test("golden: look-at solver — mira, clamps físicos e convergência", () => {
  const { golden, eye_offsets_head_local } = animeEye.gaze;
  assert.equal(golden.length, 5);
  // Offsets dos olhos batem com o contrato (esquerdo = +X, Z = frente do rosto).
  assert.deepEqual(EYE_OFFSET_LEFT, eye_offsets_head_local.left);
  assert.deepEqual(EYE_OFFSET_RIGHT, eye_offsets_head_local.right);

  for (const row of golden) {
    const gaze = solveGaze(EYE_OFFSET_LEFT, row.target);
    closeTo(gaze.yaw, row.yaw, `yaw '${row.name}'`);
    closeTo(gaze.pitch, row.pitch, `pitch '${row.name}'`);
  }

  // Clamps físicos: nunca além de 45°/35° (aceite 3 — olhar humano, não boneco).
  for (const [tx, ty, tz] of [
    [10, 0, 0.1],
    [-10, 0, 0.1],
    [0.035, 5, 1.0],
    [0.035, -5, 1.0],
    [0, 0, -10], // alvo atrás: mira embaixo/atrás, clampado
  ]) {
    const gaze = solveGaze(EYE_OFFSET_LEFT, [tx, ty, tz]);
    assert.ok(
      gaze.yaw >= -GAZE_MAX_YAW_DEGREES * 0.0174533 - 1e-9 &&
        gaze.yaw <= GAZE_MAX_YAW_DEGREES * 0.0174533 + 1e-9,
      `yaw ${gaze.yaw} fora do clamp`
    );
    assert.ok(
      gaze.pitch >= -GAZE_MAX_PITCH_DEGREES * 0.0174533 - 1e-9 &&
        gaze.pitch <= GAZE_MAX_PITCH_DEGREES * 0.0174533 + 1e-9,
      `pitch ${gaze.pitch} fora do clamp`
    );
    assert.ok(gazeAngleDegrees(gaze) <= 90, "ângulo total sempre humano (< 90°)");
  }
});

test("golden: micro-sacadas — determinísticas, ≤ amplitude e suaves", () => {
  const { saccades: golden, saccade_amplitude_range } = animeEye.gaze;
  assert.equal(golden.length, 4);
  const [minDeg, maxDeg] = saccade_amplitude_range;
  assert.ok(minDeg === 2 && maxDeg === 5, "faixa do issue: 2–5 graus");
  assert.ok(
    GAZE_SACCADE_AMPLITUDE_DEFAULT >= minDeg && GAZE_SACCADE_AMPLITUDE_DEFAULT <= maxDeg,
    "amplitude padrão dentro da faixa 2–5°"
  );

  for (const row of golden) {
    const sac = saccades(row.t);
    closeTo(sac.yaw, row.yaw, `sacada yaw t=${row.t}`);
    closeTo(sac.pitch, row.pitch, `sacada pitch t=${row.t}`);
  }

  // Varredura de 10 s a 60 fps: |sacada| ≤ amplitude e suave entre frames.
  const amplitude = GAZE_SACCADE_AMPLITUDE_DEFAULT * (Math.PI / 180);
  let previous: { yaw: number; pitch: number } | null = null;
  for (let step = 0; step < 600; step += 1) {
    const t = step / 60;
    const sac = saccades(t);
    assert.ok(sac.yaw >= -amplitude - 1e-9 && sac.yaw <= amplitude + 1e-9, `yaw t=${t}`);
    assert.ok(sac.pitch >= -amplitude - 1e-9 && sac.pitch <= amplitude + 1e-9, `pitch t=${t}`);
    if (previous) {
      const delta = Math.max(Math.abs(sac.yaw - previous.yaw), Math.abs(sac.pitch - previous.pitch));
      assert.ok(delta < 0.05, `sacada não suave entre frames (delta=${delta}) em t=${t}`);
    }
    previous = sac;
  }
});

test("golden: frame_gaze re-clampa e o tracking damping converge", () => {
  const { damping } = animeEye.gaze;
  // Damping exponencial reproduz o valor congelado após N frames.
  let current = { yaw: damping.from[0], pitch: damping.from[1] };
  const target = { yaw: damping.to[0], pitch: damping.to[1] };
  const frame = 1 / 60;
  for (let i = 0; i < damping.frames_60fps; i += 1) {
    current = dampGaze(current, target, damping.damping_per_second, frame);
  }
  closeTo(current.yaw, damping.expected[0], "damping yaw", 1e-4);
  closeTo(current.pitch, damping.expected[1], "damping pitch", 1e-4);

  // frame_gaze: mira + sacadas nunca ultrapassam o cômodo físico.
  const farTarget = [5, 5, 0.1];
  for (let step = 0; step < 120; step += 1) {
    const gaze = frameGaze(EYE_OFFSET_LEFT, farTarget, step / 60);
    const maxYaw = GAZE_MAX_YAW_DEGREES * (Math.PI / 180);
    const maxPitch = GAZE_MAX_PITCH_DEGREES * (Math.PI / 180);
    assert.ok(gaze.yaw >= -maxYaw - 1e-9 && gaze.yaw <= maxYaw + 1e-9);
    assert.ok(gaze.pitch >= -maxPitch - 1e-9 && gaze.pitch <= maxPitch + 1e-9);
  }
});

test("golden: parallax da íris — deslocamento e clamp 0..1 (aceite 1)", () => {
  const parallaxRows = animeEye.golden.filter((row) => "v_tangent" in row);
  assert.equal(parallaxRows.length, 2);
  for (const row of parallaxRows) {
    assert.ok("expected_uv" in row);
    const uv = eyeParallaxUv(row.uv, row.v_tangent, row.depth_scale);
    closeTo(uv[0], row.expected_uv[0], `parallax x '${row.name}'`, 1e-4);
    closeTo(uv[1], row.expected_uv[1], `parallax y '${row.name}'`, 1e-4);
  }
  // depth_scale 0 → identidade (frame congelado intacto).
  const identity = eyeParallaxUv([0.3, 0.7], [0.9, -0.4], 0);
  assert.deepEqual(identity, [0.3, 0.7]);
});

test("golden: highlights — mask desenhada à mão e desacoplada (aceite 2)", () => {
  const maskRows = animeEye.golden.filter((row) => !("v_tangent" in row));
  assert.equal(maskRows.length, 3);
  for (const row of maskRows) {
    assert.ok("expected_mask" in row);
    closeTo(eyeHighlightMask(row.uv), row.expected_mask, `mask '${row.name}'`, 1e-4);
  }

  // Decolpado da iluminação: o shader soma o highlight DEPOIS da iluminação —
  // com intensity 0.9 o branco é 0.9 linear (≈ 0.955 sRGB) independentemente
  // da luz: radiante mesmo em sombra total.
  const [r, g, b] = eyeHighlightRgb([0.38, 0.62], 0.9);
  assert.ok(Math.abs(r - 0.9) < 1e-9 && Math.abs(g - 0.9) < 1e-9 && Math.abs(b - 0.9) < 1e-9);
  const cel = readRepoFile("crates/anigo-renderer/shaders/cel_shading.wgsl");
  assert.match(cel, /with_rim = with_rim \+ eye_highlight_rgb\(eye_highlight, 1\.0\)/);
  // O parallax desloca o slot main (íris afundada) — mesmo slot MToon main.
  assert.match(cel, /textureSample\(main_tex, main_sampler, eye_uv\)/);
});

test("o bloco ANIGO-ANIME-EYE é byte-idêntico nos dois shaders", () => {
  const [begin, end] = animeEye.block_markers;
  const extract = (source) => source.slice(source.indexOf(begin), source.indexOf(end) + end.length);
  const cel = readRepoFile("crates/anigo-renderer/shaders/cel_shading.wgsl");
  const canonical = readRepoFile("crates/anigo-renderer/shaders/anime_eye.wgsl");
  assert.ok(cel.includes(begin) && canonical.includes(begin));
  assert.equal(extract(cel), extract(canonical), "bloco de olho anime diverge entre os shaders");
  for (const fn of animeEye.entry_functions) {
    assert.ok(cel.includes(`fn ${fn}(`), `cel_shading.wgsl sem 'fn ${fn}('`);
  }
});

test("headless/viewport carregam params8 e o Rust espelha o solver", () => {
  // Uniform: params8 em 192..208 (material 208 B = 52 floats).
  const material = RENDER_CONTRACT.uniforms.material;
  assert.equal(material.size, 208);
  const params8 = material.fields.find((field) => field.name === "params8");
  assert.ok(params8 && params8.offset === 192, "params8 ausente ou em offset errado");

  // Rust: campos do material + solver com as mesmas constantes da referência.
  const scene = readRepoFile("crates/anigo-core/src/scene.rs");
  for (const field of [
    "eye_depth_scale",
    "eye_highlight_intensity",
    "eye_enabled",
    "gaze_tracking_enabled",
    "gaze_saccade_amplitude",
    "gaze_damping",
  ]) {
    assert.ok(scene.includes(`pub ${field}:`), `scene.rs sem o campo '${field}'`);
  }
  const ik = readRepoFile("crates/anigo-ik/src/lib.rs");
  assert.match(ik, /pub struct LookAtSolver/);
  assert.match(ik, /max_yaw: 45\.0_f32\.to_radians\(\)/);
  assert.match(ik, /max_pitch: 35\.0_f32\.to_radians\(\)/);
  assert.match(ik, /saccade_amplitude: 2\.5_f32\.to_radians\(\)/);
  // Mesmas senoides do ruído (paridade TS ⇄ Rust).
  assert.match(ik, /0\.5 \* \(t \* 0\.9 \+ s\)\.sin\(\)/);
  assert.match(ik, /0\.2 \* \(t \* 2\.9 \+ 4\.3 \* s\)\.sin\(\)/);
  // Olhos são nós do grafo de cena, não ossos da paleta de skinning (24).
  const contractSkinning = RENDER_CONTRACT.skinning;
  assert.equal(contractSkinning?.joint_count, 24, "paleta de skinning permanece congelada em 24");

  // Viewport: estado + pack para o buffer de material.
  const renderer = readRepoFile("src/components/viewport/webgpu_renderer.ts");
  assert.match(renderer, /eyeDepthScale: this\.eyeDepthScale/);
  assert.match(renderer, /eyeEnabled: this\.eyeEnabled/);
});
