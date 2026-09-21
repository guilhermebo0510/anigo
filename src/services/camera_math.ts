/**
 * ANIGO — matemática de câmera/transform (P0 "Consolidar o renderer", item 4).
 *
 * Uma definição de câmera para todo o app: as mesmas convenções do núcleo Rust
 * (`crates/anigo-core/src/math.rs`): `Camera::build_view_matrix` é `look_at_rh`,
 * `build_projection_matrix` é `perspective_rh`, matrizes coluna-maior e
 * profundidade de clip 0..1.
 *
 * O viewport **não** recalcula geometria do personagem (isso é do núcleo); este
 * módulo só monta as matrizes de câmera/transform que o shader consome, a partir
 * do estado que o núcleo entrega (`Camera`/nó da cena).
 *
 * Os números são verificados contra o frame congelado do contrato do renderer
 * (`contracts/fixtures/render_contract_v1.json`) em
 * `tests/contracts/render_contract.test.ts`; o mesmo frame é conferido no Rust
 * com `glam` (`render_contract.rs`), então as duas linguagens enviam os mesmos
 * bytes de uniform.
 */

export type Vec3 = [number, number, number];

/** Matriz 4×4 em ordem de coluna (formato consumido pelos shaders). */
export type Mat4 = Float32Array;

export interface OrbitCamera {
  eye: Vec3;
  target: Vec3;
  up: Vec3;
  /** FOV vertical em **radianos** (o núcleo expõe `fov_y` já em radianos). */
  fov: number;
}

export const DEFAULT_CAMERA_NEAR = 0.05;
export const DEFAULT_CAMERA_FAR = 100.0;

function cross(a: Vec3, b: Vec3): Vec3 {
  return [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
}

function dot(a: Vec3, b: Vec3): number {
  return a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
}

function normalize(v: Vec3): Vec3 {
  const len = Math.hypot(v[0], v[1], v[2]);
  if (len < 1e-12) return [0, 0, 0];
  return [v[0] / len, v[1] / len, v[2] / len];
}

/** Matriz de visão destra (`Mat4::look_at_rh`), coluna-maior. */
export function lookAtRh(eye: Vec3, target: Vec3, up: Vec3): Mat4 {
  const z = normalize([eye[0] - target[0], eye[1] - target[1], eye[2] - target[2]]);
  let x = cross(up, z);
  let xLen = Math.hypot(x[0], x[1], x[2]);
  if (xLen < 1e-4) {
    // Câmera alinhada com o "up": escolhe um eixo auxiliar estável.
    const fallback: Vec3 = Math.abs(z[1]) < 0.9 ? [0, 1, 0] : [1, 0, 0];
    x = cross(fallback, z);
    xLen = Math.hypot(x[0], x[1], x[2]);
  }
  const right: Vec3 = [x[0] / xLen, x[1] / xLen, x[2] / xLen];
  const y = cross(z, right);

  return new Float32Array([
    right[0], y[0], z[0], 0,
    right[1], y[1], z[1], 0,
    right[2], y[2], z[2], 0,
    -dot(right, eye), -dot(y, eye), -dot(z, eye), 1,
  ]);
}

/** Projeção perspectiva destra com profundidade 0..1 (WebGPU / `perspective_rh`). */
export function perspectiveRhZeroToOne(
  fovY: number,
  aspect: number,
  near: number = DEFAULT_CAMERA_NEAR,
  far: number = DEFAULT_CAMERA_FAR
): Mat4 {
  const f = 1.0 / Math.tan(fovY / 2);
  const nf = 1.0 / (near - far);
  return new Float32Array([
    f / aspect, 0, 0, 0,
    0, f, 0, 0,
    0, 0, far * nf, -1,
    0, 0, near * far * nf, 0,
  ]);
}

/** Projeção perspectiva destra com profundidade -1..1 (WebGL2/OpenGL). */
export function perspectiveRhMinusOneToOne(
  fovY: number,
  aspect: number,
  near: number = DEFAULT_CAMERA_NEAR,
  far: number = DEFAULT_CAMERA_FAR
): Mat4 {
  const f = 1.0 / Math.tan(fovY / 2);
  const nf = 1.0 / (near - far);
  return new Float32Array([
    f / aspect, 0, 0, 0,
    0, f, 0, 0,
    0, 0, (far + near) * nf, -1,
    0, 0, 2 * near * far * nf, 0,
  ]);
}

/** Multiplicação coluna-maior: `a * b` (aplica `b` antes de `a`). */
export function multiplyMatrices(a: Mat4 | number[], b: Mat4 | number[]): Mat4 {
  const out = new Float32Array(16);
  for (let column = 0; column < 4; column++) {
    for (let row = 0; row < 4; row++) {
      let sum = 0;
      for (let k = 0; k < 4; k++) sum += a[k * 4 + row] * b[column * 4 + k];
      out[column * 4 + row] = sum;
    }
  }
  return out;
}

/** Visão-projeção na convenção de clip do backend de destino. */
export function viewProjectionMatrix(
  camera: OrbitCamera,
  aspect: number,
  options: { clipDepth?: "zero_to_one" | "minus_one_to_one"; near?: number; far?: number } = {}
): Mat4 {
  const near = options.near ?? DEFAULT_CAMERA_NEAR;
  const far = options.far ?? DEFAULT_CAMERA_FAR;
  const projection =
    options.clipDepth === "minus_one_to_one"
      ? perspectiveRhMinusOneToOne(camera.fov, aspect, near, far)
      : perspectiveRhZeroToOne(camera.fov, aspect, near, far);
  return multiplyMatrices(projection, lookAtRh(camera.eye, camera.target, camera.up));
}

/** Transform de um nó da cena: translação + rotação (quat xyzw) + escala. */
export interface NodeTransformLike {
  translation: Vec3;
  rotation_xyzw: [number, number, number, number];
  scale: Vec3;
}

export const IDENTITY_TRANSFORM: NodeTransformLike = {
  translation: [0, 0, 0],
  rotation_xyzw: [0, 0, 0, 1],
  scale: [1, 1, 1],
};

/** Matriz de modelo do transform do nó (coluna-maior, `T·R·S`). */
export function modelMatrixFromTransform(transform: NodeTransformLike = IDENTITY_TRANSFORM): Mat4 {
  const raw = transform.rotation_xyzw;
  const length = Math.hypot(raw[0], raw[1], raw[2], raw[3]) || 1;
  const qx = raw[0] / length;
  const qy = raw[1] / length;
  const qz = raw[2] / length;
  const qw = raw[3] / length;
  const [sx, sy, sz] = transform.scale;
  const [tx, ty, tz] = transform.translation;

  // Colunas da rotação (base ortonormal do quaternion).
  const c0: Vec3 = [1 - 2 * (qy * qy + qz * qz), 2 * (qx * qy + qz * qw), 2 * (qx * qz - qy * qw)];
  const c1: Vec3 = [2 * (qx * qy - qz * qw), 1 - 2 * (qx * qx + qz * qz), 2 * (qy * qz + qx * qw)];
  const c2: Vec3 = [2 * (qx * qz + qy * qw), 2 * (qy * qz - qx * qw), 1 - 2 * (qx * qx + qy * qy)];

  return new Float32Array([
    c0[0] * sx, c0[1] * sx, c0[2] * sx, 0,
    c1[0] * sy, c1[1] * sy, c1[2] * sy, 0,
    c2[0] * sz, c2[1] * sz, c2[2] * sz, 0,
    tx, ty, tz, 1,
  ]);
}

/**
 * Matriz de normais: transposta da inversa da **parte linear 3×3** do modelo,
 * "padded" a 4×4 com a linha de translação zerada.
 *
 * Mesma definição do helper Rust (`render_contract::normal_matrix`); o frame
 * congelado do contrato verifica os dois lados. Uma inversa-transposta do 4×4
 * completo carregaria a translação na última linha e quebraria a paridade dos
 * bytes do uniform.
 */
export function normalMatrixFromModel(model: Mat4 | number[]): Mat4 {
  // L em ordem de linha, montada a partir da matriz coluna-maior.
  const l: Array<Vec3> = [
    [model[0], model[4], model[8]],
    [model[1], model[5], model[9]],
    [model[2], model[6], model[10]],
  ];
  const det =
    l[0][0] * (l[1][1] * l[2][2] - l[1][2] * l[2][1]) -
    l[0][1] * (l[1][0] * l[2][2] - l[1][2] * l[2][0]) +
    l[0][2] * (l[1][0] * l[2][1] - l[1][1] * l[2][0]);
  if (Math.abs(det) < 1e-12) {
    return new Float32Array([1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1]);
  }

  const minor = (i: number, j: number): number => {
    const keepRows = [0, 1, 2].filter((index) => index !== i);
    const keepCols = [0, 1, 2].filter((index) => index !== j);
    return (
      l[keepRows[0]][keepCols[0]] * l[keepRows[1]][keepCols[1]] -
      l[keepRows[0]][keepCols[1]] * l[keepRows[1]][keepCols[0]]
    );
  };

  const out = new Float32Array(16);
  for (let column = 0; column < 3; column++) {
    for (let row = 0; row < 3; row++) {
      const sign = (row + column) % 2 === 0 ? 1 : -1;
      out[column * 4 + row] = (sign * minor(row, column)) / det;
    }
  }
  out[15] = 1;
  return out;
}
