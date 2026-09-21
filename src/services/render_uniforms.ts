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
}

/** Bloco `material` (112 B = 28 f32). */
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
