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
 * `model` são os eixos do rosto (motor cel: com paleta de skinning identidade é a
 * matriz do nó 0; com pose, o WGSL leva os eixos da junta 7 — Head — ao mundo antes).
 * column-major: a coluna 0 carrega o eixo X local
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

/**
 * Espelhamento do `u` do mapa SDF pelo lado da luz (motor cel, Fase 1): o mapa é
 * autorado para luz em +X local (theta >= 0); com theta < 0 ele é sampleado em
 * `u' = 2·center − u`. `mirrorCenter <= 0` desliga (o comportamento anterior).
 * Mesma fórmula do `face_sdf_mirror_uv` do WGSL.
 */
export function faceSdfMirrorU(u: number, theta: number, mirrorCenter: number): number {
  return mirrorCenter > 0 && theta < 0 ? 2 * mirrorCenter - u : u;
}

/**
 * Cor do rosto sombreado (o mesmo `mix(base, min(base, shadow), fator)` do WGSL):
 * a região do SDF recebe a COR DE SOMBRA do material (a mesma do terminador), nunca
 * mais clara que a banda já existente. Substitui o antigo escurecimento fixo
 * `base * 0.72`. `sdf` = 1.0 (neutro branco ancorado) devolve `base` intacto.
 */
export function faceShadowApply(
  base: [number, number, number],
  shadow: [number, number, number],
  sdf: number,
  theta: number,
  offset: number,
  smoothness: number
): [number, number, number] {
  const factor = faceSdfFactor(sdf, faceSdfThreshold(theta, offset), smoothness);
  const tone = (channel: number) => {
    const target = Math.min(base[channel], shadow[channel]);
    return base[channel] * (1.0 - factor) + target * factor;
  };
  return [tone(0), tone(1), tone(2)];
}
