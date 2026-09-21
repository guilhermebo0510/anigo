/**
 * Canonical Anime Supremo shading profile.
 *
 * UI controls, MCP patches and project files use this shape. Values are
 * normalized at the boundary and the renderer receives finite values only.
 * Colors in the profile are linear RGB; the UI may present sRGB/hex values.
 */

export const SHADING_PROFILE_VERSION = 1 as const;
export type ToonBands = 0 | 1 | 2 | 3;
export type ShadeBlendMode = "multiply" | "replace" | "lerp";
export type ToneMapOperator = "aces" | "reinhard" | "linear";
export type RgbaLinear = [number, number, number, number];

export interface ShadingProfileV1 {
  version: typeof SHADING_PROFILE_VERSION;
  baseColor: RgbaLinear;
  shadeColor: RgbaLinear;
  shadeBlend: ShadeBlendMode;
  shadowThreshold: number;
  shadowSoftness: number;
  toonBands: ToonBands;
  hueShiftDeg: number;
  shadowSaturation: number;
  aoIntensity: number;
  specular: { color: RgbaLinear; intensity: number; power: number; softness: number; size: number; offset: number };
  rim: { color: RgbaLinear; intensity: number; spread: number };
  outline: { color: RgbaLinear; widthPx: number; opacity: number; smoothness: number; depthBias: number };
  light: {
    direction: [number, number, number];
    color: RgbaLinear;
    intensity: number;
    shadowTint: RgbaLinear;
    ambientIntensity: number;
    skyColor: RgbaLinear;
    groundColor: RgbaLinear;
  };
  colorManagement: { exposureEv: number; toneMap: ToneMapOperator };
}

export const SHADING_RANGES = {
  shadowThreshold: [0, 1], shadowSoftness: [0, 0.25], hueShiftDeg: [-180, 180], shadowSaturation: [0, 2.5], aoIntensity: [0, 1],
  specularIntensity: [0, 2], specularPower: [1, 128], specularSoftness: [0.001, 1], specularSize: [0.2, 0.8], specularOffset: [-1, 1],
  rimIntensity: [0, 3], rimSpread: [0.05, 1], outlineWidthPx: [0, 32], outlineOpacity: [0, 1], outlineSmoothness: [0, 1], outlineDepthBias: [-0.01, 0.01], lightIntensity: [0, 3], ambientIntensity: [0, 2], exposureEv: [-8, 8],
} as const;

function clamp(value: number, range: readonly [number, number]): number {
  return Math.min(range[1], Math.max(range[0], Number.isFinite(value) ? value : range[0]));
}
function color(value: readonly number[] | undefined, fallback: RgbaLinear): RgbaLinear {
  return [0, 1, 2, 3].map((index) => clamp(value?.[index] ?? fallback[index], [0, index === 3 ? 1 : 16])) as RgbaLinear;
}
function direction(value: readonly number[] | undefined): [number, number, number] {
  const raw = [value?.[0] ?? 0.577, value?.[1] ?? 0.577, value?.[2] ?? 0.577].map((item) => Number.isFinite(item) ? item : 0);
  const length = Math.hypot(...raw);
  return length < 1e-6 ? [0.577, 0.577, 0.577] : [raw[0] / length, raw[1] / length, raw[2] / length];
}

export const DEFAULT_SHADING_PROFILE: ShadingProfileV1 = {
  version: 1, baseColor: [0.956, 0.823, 0.716, 1], shadeColor: [0.319, 0.284, 0.433, 1], shadeBlend: "replace",
  shadowThreshold: 0.5, shadowSoftness: 0.02, toonBands: 1, hueShiftDeg: -15, shadowSaturation: 1.1, aoIntensity: 0.85,
  specular: { color: [1, 1, 1, 1], intensity: 0.4, power: 32, softness: 0.05, size: 0.45, offset: 0 },
  rim: { color: [0.291, 0.558, 0.973, 1], intensity: 0.8, spread: 0.4 },
  outline: { color: [0.041, 0.013, 0.022, 1], widthPx: 3.5, opacity: 1, smoothness: 0, depthBias: 0 },
  light: { direction: direction(undefined), color: [1, 0.956, 0.888, 1], intensity: 1, shadowTint: [1, 1, 1, 1], ambientIntensity: 0.35, skyColor: [0.234, 0.319, 0.578, 1], groundColor: [0.051, 0.034, 0.027, 1] },
  colorManagement: { exposureEv: 0, toneMap: "aces" },
};

/** Clamps and normalizes untrusted patches without mutating the input. */
export function normalizeShadingProfile(input: Partial<ShadingProfileV1> | null | undefined): ShadingProfileV1 {
  const source = input ?? {};
  const spec: Partial<ShadingProfileV1["specular"]> = source.specular ?? {};
  const rim: Partial<ShadingProfileV1["rim"]> = source.rim ?? {};
  const outline: Partial<ShadingProfileV1["outline"]> = source.outline ?? {};
  const light: Partial<ShadingProfileV1["light"]> = source.light ?? {};
  const colorManagement: Partial<ShadingProfileV1["colorManagement"]> = source.colorManagement ?? {};
  return {
    version: 1,
    baseColor: color(source.baseColor, DEFAULT_SHADING_PROFILE.baseColor), shadeColor: color(source.shadeColor, DEFAULT_SHADING_PROFILE.shadeColor), shadeBlend: source.shadeBlend === "multiply" || source.shadeBlend === "lerp" ? source.shadeBlend : "replace",
    shadowThreshold: clamp(source.shadowThreshold ?? DEFAULT_SHADING_PROFILE.shadowThreshold, SHADING_RANGES.shadowThreshold), shadowSoftness: clamp(source.shadowSoftness ?? DEFAULT_SHADING_PROFILE.shadowSoftness, SHADING_RANGES.shadowSoftness), toonBands: source.toonBands === 0 || source.toonBands === 2 || source.toonBands === 3 ? source.toonBands : 1, hueShiftDeg: clamp(source.hueShiftDeg ?? DEFAULT_SHADING_PROFILE.hueShiftDeg, SHADING_RANGES.hueShiftDeg), shadowSaturation: clamp(source.shadowSaturation ?? DEFAULT_SHADING_PROFILE.shadowSaturation, SHADING_RANGES.shadowSaturation), aoIntensity: clamp(source.aoIntensity ?? DEFAULT_SHADING_PROFILE.aoIntensity, SHADING_RANGES.aoIntensity),
    specular: { color: color(spec.color, DEFAULT_SHADING_PROFILE.specular.color), intensity: clamp(spec.intensity ?? DEFAULT_SHADING_PROFILE.specular.intensity, SHADING_RANGES.specularIntensity), power: clamp(spec.power ?? DEFAULT_SHADING_PROFILE.specular.power, SHADING_RANGES.specularPower), softness: clamp(spec.softness ?? DEFAULT_SHADING_PROFILE.specular.softness, SHADING_RANGES.specularSoftness), size: clamp(spec.size ?? DEFAULT_SHADING_PROFILE.specular.size, SHADING_RANGES.specularSize), offset: clamp(spec.offset ?? DEFAULT_SHADING_PROFILE.specular.offset, SHADING_RANGES.specularOffset) },
    rim: { color: color(rim.color, DEFAULT_SHADING_PROFILE.rim.color), intensity: clamp(rim.intensity ?? DEFAULT_SHADING_PROFILE.rim.intensity, SHADING_RANGES.rimIntensity), spread: clamp(rim.spread ?? DEFAULT_SHADING_PROFILE.rim.spread, SHADING_RANGES.rimSpread) },
    outline: { color: color(outline.color, DEFAULT_SHADING_PROFILE.outline.color), widthPx: clamp(outline.widthPx ?? DEFAULT_SHADING_PROFILE.outline.widthPx, SHADING_RANGES.outlineWidthPx), opacity: clamp(outline.opacity ?? DEFAULT_SHADING_PROFILE.outline.opacity, SHADING_RANGES.outlineOpacity), smoothness: clamp(outline.smoothness ?? DEFAULT_SHADING_PROFILE.outline.smoothness, SHADING_RANGES.outlineSmoothness), depthBias: clamp(outline.depthBias ?? DEFAULT_SHADING_PROFILE.outline.depthBias, SHADING_RANGES.outlineDepthBias) },
    light: { direction: direction(light.direction), color: color(light.color, DEFAULT_SHADING_PROFILE.light.color), intensity: clamp(light.intensity ?? DEFAULT_SHADING_PROFILE.light.intensity, SHADING_RANGES.lightIntensity), shadowTint: color(light.shadowTint, DEFAULT_SHADING_PROFILE.light.shadowTint), ambientIntensity: clamp(light.ambientIntensity ?? DEFAULT_SHADING_PROFILE.light.ambientIntensity, SHADING_RANGES.ambientIntensity), skyColor: color(light.skyColor, DEFAULT_SHADING_PROFILE.light.skyColor), groundColor: color(light.groundColor, DEFAULT_SHADING_PROFILE.light.groundColor) },
    colorManagement: { exposureEv: clamp(colorManagement.exposureEv ?? DEFAULT_SHADING_PROFILE.colorManagement.exposureEv, SHADING_RANGES.exposureEv), toneMap: colorManagement.toneMap === "reinhard" || colorManagement.toneMap === "linear" ? colorManagement.toneMap : "aces" },
  };
}

export function srgbToLinear(value: number): number { const c = clamp(value, [0, 1]); return c <= 0.04045 ? c / 12.92 : Math.pow((c + 0.055) / 1.055, 2.4); }
export function linearToSrgb(value: number): number { const c = Math.max(0, value); return c <= 0.0031308 ? c * 12.92 : 1.055 * Math.pow(c, 1 / 2.4) - 0.055; }
export function hexToLinear(hex: string): RgbaLinear {
  const normalized = /^#?([0-9a-f]{3}|[0-9a-f]{6})$/i.exec(hex.trim());
  if (!normalized) return [...DEFAULT_SHADING_PROFILE.baseColor] as RgbaLinear;
  const raw = normalized[1].length === 3 ? normalized[1].split("").map((item) => item + item).join("") : normalized[1];
  return [srgbToLinear(parseInt(raw.slice(0, 2), 16) / 255), srgbToLinear(parseInt(raw.slice(2, 4), 16) / 255), srgbToLinear(parseInt(raw.slice(4, 6), 16) / 255), 1];
}
export function linearToHex(colorValue: readonly number[]): string {
  const toByte = (value: number) => Math.round(Math.min(1, Math.max(0, linearToSrgb(value))) * 255).toString(16).padStart(2, "0");
  return `#${toByte(colorValue[0])}${toByte(colorValue[1])}${toByte(colorValue[2])}`;
}

/** Returns the canonical toon factor for a half-Lambert value in [0,1]. */
export function evaluateToonFactor(halfLambert: number, profile: ShadingProfileV1): number {
  const p = normalizeShadingProfile(profile);
  const x = Math.min(1, Math.max(0, halfLambert));
  const edge = p.shadowThreshold;
  const smooth = Math.max(p.shadowSoftness, 0.0001);
  const smoothStep = (a: number, b: number, value: number) => { const t = Math.min(1, Math.max(0, (value - a) / Math.max(b - a, 1e-6))); return t * t * (3 - 2 * t); };
  if (p.toonBands === 0) return smoothStep(edge - 0.35 - smooth, edge + 0.35 + smooth, x);
  if (p.toonBands === 1) return smoothStep(edge - smooth, edge + smooth, x);
  if (p.toonBands === 2) return smoothStep(edge - 0.14 - smooth, edge - 0.14 + smooth, x) * 0.45 + smoothStep(edge + 0.14 - smooth, edge + 0.14 + smooth, x) * 0.55;
  return (smoothStep(edge - 0.2 - smooth, edge - 0.2 + smooth, x) + smoothStep(edge - smooth, edge + smooth, x) + smoothStep(edge + 0.2 - smooth, edge + 0.2 + smooth, x)) / 3;
}

/** Canonical neutral for `_ANIGO_COLOR`: AO, shadow shift, outline mask, rim/spec mask. */
export const ANIME_VERTEX_ATTRIBUTE_NEUTRAL: readonly [number, number, number, number] = [1, 0.5, 1, 1];
export function resolveAnimeVertexAttributes(value: readonly number[] | null | undefined): [number, number, number, number] {
  return [0, 1, 2, 3].map((index) => Number.isFinite(value?.[index]) ? value![index] : ANIME_VERTEX_ATTRIBUTE_NEUTRAL[index]) as [number, number, number, number];
}
