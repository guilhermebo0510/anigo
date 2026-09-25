/**
 * ANIGO #13 — frustum culling e projeção perspectiva/ortográfica.
 *
 * O núcleo Rust é o dono da geometria do personagem, mas o *viewport* também
 * precisa decidir o que desenhar: ele monta a própria `view_proj` (mesma
 * `camera_math.ts` do contrato) e descarta o que está fora do cone. Se as duas
 * linguagens divergissem no frustum, o mesmo nó apareceria em um renderizador e
 * sumiria no outro — por isso as propriedades testadas aqui são as mesmas
 * verificadas em `crates/anigo-core/src/math.rs` (`Frustum`, `ProjectionMode`):
 *
 *   1. os 6 planos saem da `view_proj` combinada e apontam para dentro;
 *   2. nada visível é recusado (o teste é conservador por construção);
 *   3. um objeto em `z = -100`, atrás da câmera, está fora do frustum;
 *   4. a troca perspectiva ⇄ ortográfica preserva o enquadramento na distância
 *      do alvo (é o que evita o salto visual).
 *
 * Run: node --experimental-strip-types --test tests/contracts/
 */
import test from "node:test";
import assert from "node:assert/strict";

import {
  DEFAULT_CAMERA_FAR,
  DEFAULT_CAMERA_NEAR,
  orthographicBoundsForFraming,
  orthographicRhZeroToOne,
  projectionMatrixFor,
  perspectiveRhZeroToOne,
  lookAtRh,
  multiplyMatrices,
  viewProjectionMatrix,
  type ProjectionModeWire,
} from "../../src/services/camera_math.ts";
import { readRepoFile } from "./rust_contract_source.ts";

const FOV = (45 * Math.PI) / 180;
const ASPECT = 16 / 9;
const EYE: [number, number, number] = [0, 1.5, 3.5];
const TARGET: [number, number, number] = [0, 1.0, 0.0];
const UP: [number, number, number] = [0, 1, 0];
const TOLERANCE = 1e-4;

function perspectiveMode(fovY = FOV, aspect = ASPECT): ProjectionModeWire {
  return { mode: "perspective", fov_y: fovY, aspect };
}

/** Os 6 planos (L, R, B, T, N, F) extraídos de uma `view_proj` coluna-maior. */
function frustumPlanes(viewProj: Float32Array): Array<[number, number, number, number]> {
  // Linhas da matriz (matriz coluna-maior: linha r vem de índices r, r+4, …).
  const row = (r: number): [number, number, number, number] => [
    viewProj[r],
    viewProj[4 + r],
    viewProj[8 + r],
    viewProj[12 + r],
  ];
  const add = (
    a: [number, number, number, number],
    b: [number, number, number, number]
  ): [number, number, number, number] => [a[0] + b[0], a[1] + b[1], a[2] + b[2], a[3] + b[3]];
  const sub = (
    a: [number, number, number, number],
    b: [number, number, number, number]
  ): [number, number, number, number] => [a[0] - b[0], a[1] - b[1], a[2] - b[2], a[3] - b[3]];
  const row0 = row(0);
  const row1 = row(1);
  const row2 = row(2);
  const row3 = row(3);
  const raw: Array<[number, number, number, number]> = [
    add(row3, row0),
    sub(row3, row0),
    add(row3, row1),
    sub(row3, row1),
    row2, // clip 0..1: near = row2
    sub(row3, row2),
  ];
  return raw.map(([x, y, z, w]) => {
    const length = Math.hypot(x, y, z);
    return length < 1e-9 ? [x, y, z, w] : [x / length, y / length, z / length, w / length];
  });
}

function isInside(
  planes: Array<[number, number, number, number]>,
  point: [number, number, number]
): boolean {
  return planes.every(
    ([x, y, z, w]) => x * point[0] + y * point[1] + z * point[2] + w >= -TOLERANCE
  );
}

type Vec3Tuple = [number, number, number];

function normalize(v: Vec3Tuple): Vec3Tuple {
  const length = Math.hypot(v[0], v[1], v[2]) || 1;
  return [v[0] / length, v[1] / length, v[2] / length];
}

function cross(a: Vec3Tuple, b: Vec3Tuple): Vec3Tuple {
  return [
    a[1] * b[2] - a[2] * b[1],
    a[2] * b[0] - a[0] * b[2],
    a[0] * b[1] - a[1] * b[0],
  ];
}

function projectToNdc(viewProj: Float32Array, point: [number, number, number]): [number, number, number] {
  const x = viewProj[0] * point[0] + viewProj[4] * point[1] + viewProj[8] * point[2] + viewProj[12];
  const y = viewProj[1] * point[0] + viewProj[5] * point[1] + viewProj[9] * point[2] + viewProj[13];
  const z = viewProj[2] * point[0] + viewProj[6] * point[1] + viewProj[10] * point[2] + viewProj[14];
  const w = viewProj[3] * point[0] + viewProj[7] * point[1] + viewProj[11] * point[2] + viewProj[15];
  return [x / w, y / w, z / w];
}

test("o frustum do contrato é extraído da view_proj e contém o alvo da câmera", () => {
  const viewProj = viewProjectionMatrix({ eye: EYE, target: TARGET, up: UP, fov: FOV }, ASPECT, {
    clipDepth: "zero_to_one",
  });
  const planes = frustumPlanes(viewProj);
  assert.equal(planes.length, 6);

  // O alvo (no centro da tela, à frente da câmera) está dentro dos 6 planos.
  assert.ok(isInside(planes, TARGET), "o alvo da câmera precisa estar dentro do frustum");
  // A própria lente está *fora* do near (nada antes do plano near é visível).
  assert.equal(isInside(planes, EYE), false, "a posição da câmera não é visível por ela mesma");
});

test("os planos apontam para dentro: um ponto fora da lateral é recusado", () => {
  const viewProj = viewProjectionMatrix({ eye: EYE, target: TARGET, up: UP, fov: FOV }, ASPECT, {
    clipDepth: "zero_to_one",
  });
  const planes = frustumPlanes(viewProj);
  assert.ok(isInside(planes, [0, 1, 0]), "o manequim está visível");
  assert.equal(isInside(planes, [60, 1, 0]), false, "60 unidades à direita (assunto do issue)");
  assert.equal(isInside(planes, [0, 1, -100]), false, "objeto em z = -100 fica atrás da câmera");
  assert.equal(isInside(planes, [0, 1, 200]), false, "além do plano far");
});

test("perspectiva e ortográfica produzem a mesma silhueta na distância do alvo", () => {
  // A troca de modo é o critério de aceitação #2: sem salto visual nem
  // deformação. O enquadramento perspectiva na distância do alvo define os
  // limites ortográficos.
  const distance = Math.hypot(EYE[0] - TARGET[0], EYE[1] - TARGET[1], EYE[2] - TARGET[2]);
  const bounds = orthographicBoundsForFraming(FOV, ASPECT, distance);

  const perspective = viewProjectionMatrix({ eye: EYE, target: TARGET, up: UP, fov: FOV }, ASPECT, {
    clipDepth: "zero_to_one",
  });
  const orthographic = viewProjectionMatrix({ eye: EYE, target: TARGET, up: UP, fov: FOV }, ASPECT, {
    clipDepth: "zero_to_one",
    projection: { mode: "orthographic", ...bounds },
  });

  // Um ponto no **plano do alvo** (perpendicular ao eixo da câmera) projeta no
  // mesmo NDC nos dois modos: é exatamente isso que significa "sem salto".
  // Fora desse plano as duas projeções divergem por construção — a perspectiva
  // divide pelo `w` (profundidade), a ortográfica não.
  const forward = normalize([TARGET[0] - EYE[0], TARGET[1] - EYE[1], TARGET[2] - EYE[2]]);
  const right = normalize(cross(UP, forward));
  const camUp = cross(forward, right);
  for (const [u, v] of [
    [0, 0],
    [0.4, 0],
    [-0.4, 0],
    [0, 0.3],
    [0.25, -0.2],
  ] as Array<[number, number]>) {
    const point: [number, number, number] = [
      TARGET[0] + right[0] * u + camUp[0] * v,
      TARGET[1] + right[1] * u + camUp[1] * v,
      TARGET[2] + right[2] * u + camUp[2] * v,
    ];
    const persp = projectToNdc(perspective, point);
    const ortho = projectToNdc(orthographic, point);
    assert.ok(
      Math.abs(persp[0] - ortho[0]) < TOLERANCE && Math.abs(persp[1] - ortho[1]) < TOLERANCE,
      `ponto (${u}, ${v}) projetou diferente: persp=${persp} ortho=${ortho}`
    );
  }
  // E o volume ortográfico tem a proporção da viewport (não deforma o modelo).
  assert.ok(Math.abs(bounds.right / bounds.top - ASPECT) < TOLERANCE);
  assert.ok(bounds.left === -bounds.right && bounds.bottom === -bounds.top);
});

test("a matriz ortográfica mapeia near → 0 e far → 1 (clip do wgpu)", () => {
  const matrix = orthographicRhZeroToOne(-1, 1, -1, 1, DEFAULT_CAMERA_NEAR, DEFAULT_CAMERA_FAR);
  const depth = (z: number) => matrix[10] * z + matrix[14];
  assert.ok(Math.abs(depth(-DEFAULT_CAMERA_NEAR) - 0) < TOLERANCE, "near → 0");
  assert.ok(Math.abs(depth(-DEFAULT_CAMERA_FAR) - 1) < TOLERANCE, "far → 1");

  // Sem a divisão perspectiva (w = 1): a escala não depende da profundidade.
  assert.equal(matrix[3], 0);
  assert.equal(matrix[7], 0);
  assert.equal(matrix[11], 0);
  assert.equal(matrix[15], 1);
});

test("o modo de projeção escolhe a matriz e continua sendo o mesmo vocabulário do Rust", () => {
  const near = DEFAULT_CAMERA_NEAR;
  const far = DEFAULT_CAMERA_FAR;
  const perspective = projectionMatrixFor(perspectiveMode(), "zero_to_one", near, far);
  const reference = perspectiveRhZeroToOne(FOV, ASPECT, near, far);
  for (let index = 0; index < 16; index++) {
    assert.ok(Math.abs(perspective[index] - reference[index]) < TOLERANCE);
  }

  const ortho = projectionMatrixFor(
    { mode: "orthographic", left: -2, right: 2, bottom: -1, top: 1 },
    "zero_to_one",
    near,
    far
  );
  // Uma unidade de mundo vale metade da tela na horizontal e na vertical.
  assert.ok(Math.abs(ortho[0] - 0.5) < TOLERANCE, "2 / (right - left) = 0.5");
  assert.ok(Math.abs(ortho[5] - 1.0) < TOLERANCE, "2 / (top - bottom) = 1.0");

  // O vocabulário é o do núcleo: `ProjectionMode::as_str` no Rust.
  const rustMath = readRepoFile("crates/anigo-core/src/math.rs");
  assert.match(rustMath, /Self::Perspective \{ .. \} => "perspective"/);
  assert.match(rustMath, /Self::Orthographic \{ .. \} => "orthographic"/);
  const rustCamera = readRepoFile("crates/anigo-core/src/math.rs");
  assert.match(rustCamera, /Mat4::orthographic_rh\(/, "o core precisa projetar ortográfico pelo glam");
});

test("o headless faz o culling antes de registrar a draw call e reporta na telemetria", () => {
  // O filtro roda no Rust (a GPU de verdade vive lá), então o teste de contrato
  // verifica *onde* ele entra: antes dos buffers e antes do `draw_indexed`.
  const headless = readRepoFile("crates/anigo-renderer/src/headless.rs");
  assert.match(headless, /culled_draw_calls: u32/, "a métrica precisa existir no RenderMetrics");
  assert.match(headless, /Frustum::from_view_projection\(&view_proj\)/);
  assert.match(headless, /if !volume\.is_visible\(&frustum\)/);
  assert.match(headless, /if let Some\(local_bounds\) = mesh\.local_bounds\(\)/);

  const cullIndex = headless.indexOf("if !volume.is_visible(&frustum)");
  const drawIndex = headless.indexOf("render_pass.draw_indexed(");
  assert.ok(cullIndex !== -1 && drawIndex !== -1 && cullIndex < drawIndex, "o culling precede o draw");

  const culling = readRepoFile("crates/anigo-renderer/src/culling.rs");
  assert.match(culling, /pub fn cull_volumes\(/, "o relatório de culling é público e testável");
});
