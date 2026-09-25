/**
 * ANIGO — sombra facial SDF (Fase 2 #17): implementação de referência em TS.
 *
 * O WGSL que desenha (o bloco ANIGO-FACE-SDF em `cel_shading.wgsl`, cópia
 * byte-idêntica do canônico `face_sdf.wgsl`) implementa exatamente estas
 * fórmulas. Este módulo existe para que a matemática tenha uma superfície
 * testável fora da GPU: os números dourados do contrato
 * (`render_contract.face_sdf.golden`, ângulos 0°/45°/90°/135°) foram
 * produzidos por uma implementação independente e precisam ser reproduzidos
 * aqui dentro de 1e-5 — o mesmo mecanismo de paridade do reference frame.
 */

import type { Mat4 } from "./camera_math";

/**
 * Azimut da luz no espaço local da cabeça (theta).
 *
 * `model` é a matriz do nó 0, column-major: a coluna 0 carrega o eixo X local
 * e a coluna 2 o eixo Z local, em coordenadas de mundo — para escala positiva
 * a direção é preservada, então não precisamos da inversa (o WGSL faz o
 * mesmo com `camera.model[0]`/`camera.model[2]`). O eixo Z local é a frente
 * do rosto: `theta = atan2(dot(L, eixoX_local), dot(L, eixoZ_local))`,
 * 0 = luz frontal.
 */
export function faceSdfTheta(model: Mat4 | ArrayLike<number>, lightDir: [number, number, number]): number {
  // coluna 0 = [m0, m1, m2], coluna 2 = [m8, m9, m10] (column-major)
  const lx = lightDir[0] * model[0] + lightDir[1] * model[1] + lightDir[2] * model[2];
  const lz = lightDir[0] * model[8] + lightDir[1] * model[9] + lightDir[2] * model[10];
  const len = Math.hypot(lx, lz);
  if (len < 1e-6) return 0;
  return Math.atan2(lx / len, lz / len);
}

/**
 * Threshold do SDF deslocado pelo azimut: luz frontal (theta ≈ 0) → 0.5 +
 * offset; luz traseira (|theta| ≈ π) → 0.75 + offset. `offset` é o controle
 * `face_shadow_offset` do material.
 */
export function faceSdfThreshold(theta: number, offset: number): number {
  const lightFront = Math.cos(theta) * 0.5 + 0.5;
  return 0.5 + (1.0 - lightFront) * 0.25 + offset;
}

/**
 * Fator de sombra facial em um pixel: 1 = totalmente na sombra, 0 = fora da
 * região do SDF. `sdf` é o canal R do mapa (0 = centro, 1 = fora);
 * `softness` é o controle `face_shadow_smoothness`.
 */
export function faceSdfFactor(sdf: number, threshold: number, softness: number): number {
  const s = Math.max(softness, 0.001);
  const t = Math.min(Math.max((sdf - (threshold - s)) / (2 * s), 0), 1);
  return 1.0 - t * t * (3.0 - 2.0 * t);
}

/** 0..1 — quanto da cor base fica escurecida pela sombra facial (0.72 = 28% de escurecimento). */
export const FACE_SDF_DARKENING = 0.72;

/**
 * Cor base final após a sombra facial (o mesmo `mix(base, base*0.72, fator)`
 * do WGSL). `sdf` = 1.0 (neutro branco ancorado) devolve `base` intacto.
 */
export function faceShadowApply(
  base: [number, number, number],
  sdf: number,
  theta: number,
  offset: number,
  smoothness: number
): [number, number, number] {
  const factor = faceSdfFactor(sdf, faceSdfThreshold(theta, offset), smoothness);
  const k = 1.0 - factor * (1.0 - FACE_SDF_DARKENING);
  return [base[0] * k, base[1] * k, base[2] * k];
}
