/**
 * ANIGO — motor de olhos/íris (Fase 2 #43): implementação de referência em TS.
 *
 * Espelha **exatamente** o solver Rust (`crates/anigo-ik/src/lib.rs`,
 * `LookAtSolver`) — mesmo modelo matemático, mesmo clamps, mesmas senoides.
 * Os números dourados do contrato (`render_contract.anime_eye`) foram
 * produzidos por uma implementação independente e precisam ser reproduzidos
 * aqui dentro de 1e-5; o WGSL (bloco ANIGO-ANIME-EYE, byte-idêntico em
 * anime_eye.wgsl e cel_shading.wgsl) implementa as mesmas fórmulas de
 * parallax/highlights (hash congelado + byte-identity conferidos por
 * `npm run check:wgsl`).
 *
 * Convenção: o olhar é o eixo +Z local do olho.
 *   yaw   = rotação em volta do Y local (positivo = olhar para o +X)
 *   pitch = rotação em volta do X local (positivo = olhar para cima)
 * Os offsets dos olhos são no espaço LOCAL DA CABEÇA (Z = frente do rosto);
 * como o olhar é resolvido nesse espaço, a rotação acompanha a cabeça de graça.
 */

import type { Mat4, Vec3 } from "./camera_math";

/** Offset do olho em relação ao osso da cabeça (congelado no contrato). */
export const EYE_OFFSET_LEFT: Vec3 = [0.035, -0.01, 0.09];
export const EYE_OFFSET_RIGHT: Vec3 = [-0.035, -0.01, 0.09];

/** Clamp físico do olhar (padrões do contrato). */
export const GAZE_MAX_YAW_DEGREES = 45;
export const GAZE_MAX_PITCH_DEGREES = 35;
/** Amplitude padrão das micro-sacadas em graus (faixa do issue: 2–5). */
export const GAZE_SACCADE_AMPLITUDE_DEFAULT = 2.5;
/** Damping padrão do tracking (1/s). */
export const GAZE_DAMPING_DEFAULT = 6.0;
/** Semente padrão do ruído de sacadas (paridade com o Rust). */
export const GAZE_SACCADE_SEED = 1.23;

const DEG2RAD = Math.PI / 180;

export interface GazeYawPitch {
  /** Radianos — rotação em volta do Y local (+ = olhar para o +X). */
  yaw: number;
  /** Radianos — rotação em volta do X local (+ = olhar para cima). */
  pitch: number;
}

const ZERO_GAZE: GazeYawPitch = { yaw: 0, pitch: 0 };

/** Ângulo total do olhar em graus (módulo). */
export function gazeAngleDegrees(gaze: GazeYawPitch): number {
  return Math.hypot(gaze.yaw, gaze.pitch) / DEG2RAD;
}

/**
 * Resolve o olhar do `eyeLocal` até `targetLocal` (ambos no espaço local da
 * cabeça), com clamps físicos. Mirando à frente → yaw/pitch ≈ 0.
 */
export function solveGaze(
  eyeLocal: Vec3,
  targetLocal: Vec3,
  maxYawDegrees = GAZE_MAX_YAW_DEGREES,
  maxPitchDegrees = GAZE_MAX_PITCH_DEGREES
): GazeYawPitch {
  const vx = targetLocal[0] - eyeLocal[0];
  const vy = targetLocal[1] - eyeLocal[1];
  const vz = targetLocal[2] - eyeLocal[2];
  const yawUnclamped = Math.atan2(vx, vz);
  const horizontal = Math.hypot(vx, vz);
  const pitchUnclamped = horizontal < 1e-6 ? 0 : Math.atan2(vy, horizontal);
  return {
    yaw: Math.min(Math.max(yawUnclamped, -maxYawDegrees * DEG2RAD), maxYawDegrees * DEG2RAD),
    pitch: Math.min(
      Math.max(pitchUnclamped, -maxPitchDegrees * DEG2RAD),
      maxPitchDegrees * DEG2RAD
    ),
  };
}

/**
 * Alvo em coordenadas de mundo → espaço local da cabeça (rotação inversa,
 * só o 3×3: a translação é removida pela diferença de posição).
 */
export function targetToHeadLocal(
  headWorldRotation: Mat4 | number[],
  headWorldPosition: Vec3,
  targetWorld: Vec3
): Vec3 {
  const dx = targetWorld[0] - headWorldPosition[0];
  const dy = targetWorld[1] - headWorldPosition[1];
  const dz = targetWorld[2] - headWorldPosition[2];
  // R^T (column-major: linhas de R^T = colunas de R)
  const rx = headWorldRotation[0] * dx + headWorldRotation[4] * dy + headWorldRotation[8] * dz;
  const ry = headWorldRotation[1] * dx + headWorldRotation[5] * dy + headWorldRotation[9] * dz;
  const rz = headWorldRotation[2] * dx + headWorldRotation[6] * dy + headWorldRotation[10] * dz;
  return [rx, ry, rz];
}

/**
 * Micro-sacadas: ruído temporariamente coerente (3 senoides de frequências
 * incomensuráveis, |ruído| ≤ 1.0), multiplicado pela amplitude. Determinístico
 * em (t, seed) — idêntico ao Rust.
 */
export function saccades(
  timeSeconds: number,
  amplitudeDegrees = GAZE_SACCADE_AMPLITUDE_DEFAULT,
  seed = GAZE_SACCADE_SEED
): GazeYawPitch {
  const t = timeSeconds;
  const s = seed;
  const amplitude = amplitudeDegrees * DEG2RAD;
  const ny =
    0.5 * Math.sin(t * 0.9 + s) +
    0.3 * Math.sin(t * 1.7 + 2.1 * s) +
    0.2 * Math.sin(t * 2.3 + 3.7 * s);
  const np =
    0.5 * Math.sin(t * 1.1 + 1.3 * s) +
    0.3 * Math.sin(t * 1.9 + 2.7 * s) +
    0.2 * Math.sin(t * 2.9 + 4.3 * s);
  return { yaw: ny * amplitude, pitch: np * amplitude };
}

/** Giro total do frame: mira + sacadas, re-clampado ao cômodo físico. */
export function frameGaze(
  eyeLocal: Vec3,
  targetLocal: Vec3,
  timeSeconds: number,
  amplitudeDegrees = GAZE_SACCADE_AMPLITUDE_DEFAULT,
  seed = GAZE_SACCADE_SEED
): GazeYawPitch {
  const aim = solveGaze(eyeLocal, targetLocal);
  const sac = saccades(timeSeconds, amplitudeDegrees, seed);
  const maxYaw = GAZE_MAX_YAW_DEGREES * DEG2RAD;
  const maxPitch = GAZE_MAX_PITCH_DEGREES * DEG2RAD;
  return {
    yaw: Math.min(Math.max(aim.yaw + sac.yaw, -maxYaw), maxYaw),
    pitch: Math.min(Math.max(aim.pitch + sac.pitch, -maxPitch), maxPitch),
  };
}

/**
 * Suavização exponencial do tracking (criticamente amortecida):
 * `current += (target − current) × (1 − e^(−damping × dt))`. damping 1/s:
 * maior = mais rígido; a 60 fps e damping 6 o olhar converge em ~0.5 s.
 */
export function dampGaze(
  current: GazeYawPitch,
  target: GazeYawPitch,
  dampingPerSecond = GAZE_DAMPING_DEFAULT,
  deltaSeconds: number
): GazeYawPitch {
  const k = 1 - Math.exp(-Math.max(dampingPerSecond, 0) * Math.max(deltaSeconds, 0));
  return {
    yaw: current.yaw + (target.yaw - current.yaw) * k,
    pitch: current.pitch + (target.pitch - current.pitch) * k,
  };
}

/**
 * Quat [x, y, z, w] do olhar: yaw em volta de Y, depois pitch em volta de X
 * (sinal invertido — olhar para cima é pitch positivo). Mesma construção do
 * Rust (`GazeYawPitch::to_quaternion`).
 */
export function gazeQuaternion(gaze: GazeYawPitch): [number, number, number, number] {
  const yawQ: [number, number, number, number] = [
    0,
    Math.sin(gaze.yaw / 2),
    0,
    Math.cos(gaze.yaw / 2),
  ];
  const pitchQ: [number, number, number, number] = [
    Math.sin(-gaze.pitch / 2),
    0,
    0,
    Math.cos(-gaze.pitch / 2),
  ];
  // q = yawQ * pitchQ
  const [ax, ay, az, aw] = yawQ;
  const [bx, by, bz, bw] = pitchQ;
  return [
    aw * bx + ax * bw + ay * bz - az * by,
    aw * by - ax * bw + ay * bw + az * bx,
    aw * bz + ax * by - ay * bx + az * bw,
    aw * bw - ax * bx - ay * by - az * bz,
  ];
}

// ─────────────────────────────────────────────────────────────────────────────
// Olho anime (shader) — referência TS do bloco ANIGO-ANIME-EYE (WGSL)
// ─────────────────────────────────────────────────────────────────────────────

function smoothstep(edge0: number, edge1: number, x: number): number {
  const t = Math.min(Math.max((x - edge0) / (edge1 - edge0), 0), 1);
  return t * t * (3 - 2 * t);
}

/**
 * Parallax da íris: `UV + V_tangent.xy × depth_scale` (clamp 0..1) — a fórmula
 * do issue. depth_scale 0 → identidade (frame congelado intacto).
 */
export function eyeParallaxUv(
  uv: [number, number],
  viewTangentXy: [number, number],
  depthScale: number
): [number, number] {
  const x = Math.min(Math.max(uv[0] + viewTangentXy[0] * depthScale, 0), 1);
  const y = Math.min(Math.max(uv[1] + viewTangentXy[1] * depthScale, 0), 1);
  return [x, y];
}

/**
 * Mask dos highlights desenhados à mão (0..1): elipse principal no canto
 * superior + ponto secundário no canto inferior (85% da intensidade).
 */
export function eyeHighlightMask(uv: [number, number]): number {
  const dxm = uv[0] - 0.38;
  const dym = (uv[1] - 0.62) * 0.72;
  const dMain = Math.hypot(dxm, dym);
  const main = 1 - smoothstep(0.075, 0.125, dMain);
  const ds = Math.hypot(uv[0] - 0.68, uv[1] - 0.34);
  const second = (1 - smoothstep(0.028, 0.055, ds)) * 0.85;
  return Math.max(main, second);
}

/**
 * Cor dos highlights (branco linear × mask × intensidade) — somada DEPOIS da
 * iluminação no shader: o branco dos olhos permanece em sombra total.
 */
export function eyeHighlightRgb(
  uv: [number, number],
  intensity: number
): [number, number, number] {
  const value = eyeHighlightMask(uv) * intensity;
  return [value, value, value];
}

export { ZERO_GAZE };
