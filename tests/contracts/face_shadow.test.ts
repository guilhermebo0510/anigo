/**
 * ANIGO — Sombra Facial SDF (Fase 2 #17): golden test + paridade de contrato.
 *
 * Critério de aceite #3 do issue: progressão da sombra em 0°, 45°, 90° e
 * 135°. Os números dourados são independentes (produzidos fora do pipeline,
 * luz rotacionando ao redor de Y, modelo identidade, offset 0) e congelados
 * no contrato `render_contract.face_sdf.golden`. A implementação de
 * referência (src/services/face_shadow.ts) precisa reproduzi-los; o WGSL
 * (bloco ANIGO-FACE-SDF, byte-idêntico em face_sdf.wgsl e cel_shading.wgsl)
 * implementa as mesmas fórmulas — hash congelado + byte-identity conferidos
 * por `npm run check:wgsl`.
 */
import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import {
  faceSdfTheta,
  faceSdfThreshold,
  faceSdfFactor,
  faceShadowApply,
  FACE_SDF_DARKENING,
} from "../../src/services/face_shadow";
import { RENDER_CONTRACT } from "../../src/contracts/render_contract.v1";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "..");
const readRepoFile = (relative) => fs.readFileSync(path.join(root, relative), "utf8");

const faceSdf = RENDER_CONTRACT.face_sdf;
assert.ok(faceSdf, "contrato precisa declarar a seção face_sdf");

// Matriz identidade column-major (nó 0 sem transform: eixo Z = frente do rosto).
const IDENTITY = [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1];

function closeTo(actual, expected, label, tolerance = 1e-5) {
  assert.ok(
    typeof actual === "number" && Number.isFinite(actual) && Math.abs(actual - expected) <= tolerance,
    `${label}: esperado ${expected}, veio ${actual}`
  );
}

test("golden: progressão da sombra em 0°, 45°, 90° e 135° (aceite do issue)", () => {
  assert.equal(faceSdf.golden.length, 4);
  const angles = faceSdf.golden.map((row) => row.azimuth_degrees);
  assert.deepEqual(angles, [0, 45, 90, 135]);

  for (const row of faceSdf.golden) {
    const radians = (row.azimuth_degrees * Math.PI) / 180;
    // Luz rotacionando ao redor de Y, modelo identidade → L = (sen az, 0, cos az).
    const lightDir: [number, number, number] = [Math.sin(radians), 0, Math.cos(radians)];
    const theta = faceSdfTheta(IDENTITY, lightDir);
    const threshold = faceSdfThreshold(theta, 0);
    const factor = faceSdfFactor(0.5, threshold, 0.05);

    closeTo(theta, row.theta, `theta @ ${row.azimuth_degrees}°`);
    closeTo(Math.cos(theta) * 0.5 + 0.5, row.light_front, `light_front @ ${row.azimuth_degrees}°`);
    closeTo(threshold, row.threshold, `threshold @ ${row.azimuth_degrees}°`);
    closeTo(factor, row.factor_sdf_half, `fator (sdf=0.5) @ ${row.azimuth_degrees}°`, 1e-4);
  }

  // A progressão é monotônica: luz lateral/traseira abre a banda de sombra.
  const thresholds = faceSdf.golden.map((row) => row.threshold);
  for (let i = 1; i < thresholds.length; i += 1) {
    assert.ok(thresholds[i] > thresholds[i - 1], "threshold precisa crescer com o azimut");
  }
});

test("o WGSL do cel usa as mesmas fórmulas do bloco canônico", () => {
  const cel = readRepoFile("crates/anigo-renderer/shaders/cel_shading.wgsl");
  const canonical = readRepoFile("crates/anigo-renderer/shaders/face_sdf.wgsl");

  // Projeção angular: theta = atan2(dot(L, eixoX_local), dot(L, eixoZ_local))
  // — as colunas 0 e 2 da matriz de modelo (frente do rosto = eixo Z local).
  assert.match(cel, /face_sdf_theta\(camera\.model\[0\]\.xyz,\s*camera\.model\[2\]\.xyz,\s*L\)/);
  // Threshold desloca até 0.25 com o azimut (mesma constante do bloco).
  assert.match(canonical, /return 0\.5 \+ \(1\.0 - light_front\) \* 0\.25 \+ offset;/);
  // Escurecimento do passe de cel (mesma constante do serviço TS).
  assert.ok(cel.includes(`base_cel * ${FACE_SDF_DARKENING}`), "cel precisa escurecer com 0.72");
  assert.equal(FACE_SDF_DARKENING, faceSdf.darkening, "constante do escurecimento diverge do contrato");
  // Gating por material: sem o flag o bloco não roda (frame congelado intacto).
  assert.match(cel, /if \(material\.params7\.z > 0\.5\)/);
});

test("headless ancora o SDF no neutro 1x1 e declara os bindings 16/17", () => {
  const headless = readRepoFile("crates/anigo-renderer/src/headless.rs");
  for (const binding of [16, 17]) {
    assert.ok(
      headless.includes(`binding: ${binding},`),
      `layout/binding group do cel sem o binding ${binding} (SDF facial)`
    );
  }
  // O slot SDF recebe o mesmo neutro 1x1 branco dos slots MToon (R=1 → fator 0).
  const anchorCount = headless.match(
    /binding: 16,\s*resource: wgpu::BindingResource::TextureView\(&self\.mtoon_neutral_view\)/g
  )?.length ?? 0;
  assert.equal(anchorCount, 2, "o SDF precisa estar ancorado nos 2 bind groups do cel");
});

test("o viewport vincula os bindings 16/17 no bind group do cel", () => {
  const renderer = readRepoFile("src/components/viewport/webgpu_renderer.ts");
  assert.match(renderer, /binding: 16, resource: mtoonNeutralView/);
  assert.match(renderer, /binding: 17, resource: mtoonNeutralSampler/);
  // Parâmetros fluem do estado do renderer para o buffer de material.
  assert.match(renderer, /faceShadowOffset: this\.faceShadowOffset/);
  assert.match(renderer, /faceSdfEnabled: this\.faceSdfEnabled/);
});

test("o neutral 1x1 devolve a cor base intacta (frame congelado)", () => {
  // sdf = 1.0 (branco ancorado) → fator 0 para qualquer threshold calibrado.
  for (const azimuth of [0, 45, 90, 135, 180]) {
    const radians = (azimuth * Math.PI) / 180;
    const theta = faceSdfTheta(IDENTITY, [Math.sin(radians), 0, Math.cos(radians)]);
    const [r, g, b] = faceShadowApply([1.0, 0.4, 0.8], 1.0, theta, 0.2, 0.05);
    assert.equal(r, 1.0);
    assert.equal(g, 0.4);
    assert.equal(b, 0.8);
  }
  // sdf = 0.0 (centro da região) com luz traseira → escurecimento máximo.
  const [r, g, b] = faceShadowApply([1.0, 1.0, 1.0], 0.0, Math.PI, 0.2, 0.05);
  assert.ok(Math.abs(r - FACE_SDF_DARKENING) < 1e-9);
  assert.ok(Math.abs(g - FACE_SDF_DARKENING) < 1e-9);
  assert.ok(Math.abs(b - FACE_SDF_DARKENING) < 1e-9);
});

test("o bloco compartilhado existe nos dois shaders (byte-identity)", () => {
  const [begin, end] = faceSdf.block_markers;
  for (const shaderName of faceSdf.shared_by) {
    const shader = RENDER_CONTRACT.shaders.find((candidate) => candidate.name === shaderName);
    assert.ok(shader, `shader '${shaderName}' não está no contrato`);
    const source = readRepoFile(shader.path);
    const start = source.indexOf(begin);
    const stop = source.indexOf(end);
    assert.ok(start >= 0 && stop > start, `${shader.path} sem o bloco ${begin}`);
  }
  const cel = readRepoFile("crates/anigo-renderer/shaders/cel_shading.wgsl");
  const canonical = readRepoFile("crates/anigo-renderer/shaders/face_sdf.wgsl");
  const extract = (source) =>
    source.slice(source.indexOf(begin), source.indexOf(end) + end.length);
  assert.equal(extract(cel), extract(canonical), "bloco de face SDF diverge entre os shaders");
  for (const fn of faceSdf.entry_functions) {
    assert.ok(cel.includes(`fn ${fn}(`), `cel_shading.wgsl sem 'fn ${fn}('`);
  }
});
