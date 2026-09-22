/**
 * ANIGO — empacotamento dos uniforms do renderer (P0 "Consolidar o renderer",
 * item 3: unificar buffers e uniforms).
 *
 * Os offsets e tamanhos vêm do contrato (`contracts/fixtures/render_contract_v1.json`),
 * não de números mágicos espalhados pelo código: se o layout mudar, o contrato
 * muda junto e o teste cruzado quebra antes de virar diferença visual.
 *
 * Estes mesmos floats são conferidos contra o frame congelado do contrato, e o
 * renderer Rust produz os mesmos bytes a partir de `uniforms.rs`
 * (`CameraUniform`/`LightUniform`/`MaterialUniform`/`OutlineUniform`) — é o que
 * garante paridade entre headless e viewport sem GPU.
 */

import { uniformFloats, uniformOffset } from "../contracts/render_contract.v1";
import type { Mat4, Vec3 } from "./camera_math";

function writeVec(target: Float32Array, block: string, field: string, values: ArrayLike<number>): void {
  target.set(values, uniformOffset(block, field) / 4);
}

export interface CameraUniformInput {
  /** Matriz visão-projeção (16 floats, coluna-maior). */
  viewProj: Mat4 | ArrayLike<number>;
  eye: Vec3;
  /** Modelo do nó (identidade quando não há transform de nó — P0 item 5). */
  model?: Mat4 | ArrayLike<number>;
  normalMat?: Mat4 | ArrayLike<number>;
}

export const IDENTITY_MAT4 = new Float32Array([1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1]);

/** Bloco `camera` (208 B = 52 f32): view_proj + camera_pos + model + normal_mat. */
export function cameraUniformFloats(input: CameraUniformInput): Float32Array {
  const data = new Float32Array(uniformFloats("camera"));
  writeVec(data, "camera", "view_proj", input.viewProj);
  writeVec(data, "camera", "camera_pos", [input.eye[0], input.eye[1], input.eye[2], 1.0]);
  writeVec(data, "camera", "model", input.model ?? IDENTITY_MAT4);
  writeVec(data, "camera", "normal_mat", input.normalMat ?? IDENTITY_MAT4);
  return data;
}

export interface LightUniformInput {
  direction: Vec3;
  intensity: number;
  color: Vec3;
  ambientIntensity: number;
  shadowColor: Vec3;
  shadowSaturation: number;
  ambientSky: Vec3;
  ambientGround: Vec3;
}

/** Bloco `light` (80 B = 20 f32). */
export function lightUniformFloats(input: LightUniformInput): Float32Array {
  const data = new Float32Array(uniformFloats("light"));
  writeVec(data, "light", "direction", [...input.direction, input.intensity]);
  writeVec(data, "light", "color", [...input.color, input.ambientIntensity]);
  writeVec(data, "light", "shadow_color", [...input.shadowColor, input.shadowSaturation]);
  writeVec(data, "light", "ambient_sky", [...input.ambientSky, 1.0]);
  writeVec(data, "light", "ambient_ground", [...input.ambientGround, 1.0]);
  return data;
}

export interface MaterialUniformInput {
  baseColor: [number, number, number, number];
  shadeColor: [number, number, number, number];
  specularColor: [number, number, number, number];
  rimColor: [number, number, number, number];
  shadowThreshold: number;
  shadowSmoothness: number;
  specIntensity: number;
  specPower: number;
  rimIntensity: number;
  rimSpread: number;
  /** Graus — convertido para radianos no bloco (`params2.z`). */
  hueShiftDegrees: number;
  toonSteps: number;
  specularSoftness: number;
  specularOffset: number;
  specularSize: number;
  aoIntensity: number;
  // Fase 2 (#18): material anime VRoid/MToon — opcionais com o default do
  // `StylizedMaterial`: slots off, sem emissão/matcap, shade_toony = true.
  mtoonEmissionColor?: [number, number, number, number];
  mtoonEmissionIntensity?: number;
  mtoonSecondShadeShift?: number;
  mtoonSecondShadeSoftness?: number;
  mtoonMatcapIntensity?: number;
  mtoonMainTextureEnabled?: boolean;
  mtoonShadeTextureEnabled?: boolean;
  mtoonSecondShadeTextureEnabled?: boolean;
  mtoonEmissionTextureEnabled?: boolean;
  mtoonMatcapEnabled?: boolean;
  /** 0 = normal (mult), 1 = additive. */
  mtoonMatcapMode?: number;
  mtoonShadeToony?: boolean;
  // Fase 2 (#17): sombra facial SDF (default = off, smoothness 0.05)
  faceShadowOffset?: number;
  faceShadowSmoothness?: number;
  faceSdfEnabled?: boolean;
  // Fase 2 (#43): olho anime (default = off)
  eyeDepthScale?: number;
  eyeHighlightIntensity?: number;
  eyeEnabled?: boolean;
}

/** Bloco `material` (208 B = 52 f32). */
export function materialUniformFloats(input: MaterialUniformInput): Float32Array {
  const data = new Float32Array(uniformFloats("material"));
  writeVec(data, "material", "base_color", input.baseColor);
  writeVec(data, "material", "shade_color", input.shadeColor);
  writeVec(data, "material", "specular_color", input.specularColor);
  writeVec(data, "material", "rim_color", input.rimColor);
  writeVec(data, "material", "params", [
    input.shadowThreshold,
    input.shadowSmoothness,
    input.specIntensity,
    input.specPower,
  ]);
  writeVec(data, "material", "params2", [
    input.rimIntensity,
    input.rimSpread,
    (input.hueShiftDegrees * Math.PI) / 180.0,
    input.toonSteps,
  ]);
  writeVec(data, "material", "params3", [
    input.specularSoftness,
    input.specularOffset,
    input.specularSize,
    input.aoIntensity,
  ]);
  // Fase 2 (#18): MToon — emission, segundo shade, matcap e flags de textura.
  writeVec(data, "material", "emission_color", input.mtoonEmissionColor ?? [0.0, 0.0, 0.0, 0.0]);
  writeVec(data, "material", "params4", [
    input.mtoonEmissionIntensity ?? 0.0,
    input.mtoonSecondShadeShift ?? 0.0,
    input.mtoonSecondShadeSoftness ?? 0.05,
    input.mtoonMatcapIntensity ?? 0.0,
  ]);
  writeVec(data, "material", "params5", [
    (input.mtoonMainTextureEnabled ?? false) ? 1.0 : 0.0,
    (input.mtoonShadeTextureEnabled ?? false) ? 1.0 : 0.0,
    (input.mtoonSecondShadeTextureEnabled ?? false) ? 1.0 : 0.0,
    (input.mtoonEmissionTextureEnabled ?? false) ? 1.0 : 0.0,
  ]);
  writeVec(data, "material", "params6", [
    (input.mtoonMatcapEnabled ?? false) ? 1.0 : 0.0,
    input.mtoonMatcapMode ?? 0,
    (input.mtoonShadeToony ?? true) ? 1.0 : 0.0,
    0.0,
  ]);
  // Fase 2 (#17): SDF facial — off por padrão (mapa ancorado no neutro 1x1)
  writeVec(data, "material", "params7", [
    input.faceShadowOffset ?? 0.0,
    input.faceShadowSmoothness ?? 0.05,
    (input.faceSdfEnabled ?? false) ? 1.0 : 0.0,
    0.0,
  ]);
  // Fase 2 (#43): olho anime — off por padrão (eye_uv = in.uv, highlight 0)
  writeVec(data, "material", "params8", [
    input.eyeDepthScale ?? 0.0,
    input.eyeHighlightIntensity ?? 0.0,
    (input.eyeEnabled ?? false) ? 1.0 : 0.0,
    0.0,
  ]);
  return data;
}

export interface OutlineUniformInput {
  color: [number, number, number, number];
  width: number;
  /** Aspecto do alvo (o shader precisa para manter a espessura em pixels). */
  aspect: number;
  depthBias: number;
  opacity: number;
  smoothness: number;
}

/** Bloco `outline` (48 B = 12 f32). */
export function outlineUniformFloats(input: OutlineUniformInput): Float32Array {
  const data = new Float32Array(uniformFloats("outline"));
  writeVec(data, "outline", "color", input.color);
  writeVec(data, "outline", "params", [input.width, input.aspect, input.depthBias, input.opacity]);
  writeVec(data, "outline", "params2", [input.smoothness, 0.0, 0.0, 0.0]);
  return data;
}

// ─── Fase 2 (#53): Depth of Field cinematográfico (Anime Bokeh DoF) ────────

/**
 * Inputs do buffer `DofUniform` (48 B / 12 floats — bloco `dof` do contrato):
 * params = [focus_distance (m), f_number, bokeh_shape (0 círculo/1 hex),
 * focal_m (m)]; resolution = [width_px, height_px, z_near (m), z_far (m)];
 * limits = [max_radius_px, sensor_height_m (0.024), reserved, reserved].
 *
 * O MESMO empacotamento roda no headless Rust (`headless.rs`) — o golden do
 * contrato (cinematography) garante a paridade byte a byte.
 */
export interface DofUniformInput {
  /** Distância de foco (m) — o plano milimetricamente nítido. */
  focusDistance: number;
  /** Número f (abertura) — menor = mais bokeh. */
  fNumber: number;
  /** 0 = bokeh circular, 1 = hexagonal. */
  bokehShape: number;
  /** Distância focal ativa em mm (vira metros no buffer). */
  focalMm: number;
  /** Raio máximo do bokeh em px (teto de custo). */
  maxRadiusPx: number;
  /** Dimensões do alvo em px. */
  widthPx: number;
  heightPx: number;
  /** Plano próximo/longe (m) para linearizar a profundidade. */
  zNear: number;
  zFar: number;
}

export function dofUniformFloats(input: DofUniformInput): Float32Array {
  const data = new Float32Array(uniformFloats("dof"));
  writeVec(data, "dof", "params", [
    input.focusDistance,
    input.fNumber,
    input.bokehShape,
    input.focalMm / 1000.0,
  ]);
  writeVec(data, "dof", "resolution", [input.widthPx, input.heightPx, input.zNear, input.zFar]);
  writeVec(data, "dof", "limits", [input.maxRadiusPx, 0.024, 0.0, 0.0]);
  return data;
}

/** Cabeçalho do compute de morphs esparsos (`SparseMorphHeader`, 4×u32). */
export function sparseMorphHeader(
  activeChannelCount: number,
  totalVertexCount: number,
  totalDeltaCount: number
): Uint32Array {
  const data = new Uint32Array(uniformFloats("sparse_morph_header"));
  data[uniformOffset("sparse_morph_header", "active_channel_count") / 4] = activeChannelCount;
  data[uniformOffset("sparse_morph_header", "total_vertex_count") / 4] = totalVertexCount;
  data[uniformOffset("sparse_morph_header", "total_delta_count") / 4] = totalDeltaCount;
  return data;
}
