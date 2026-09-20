// P1-13 RenderConfig único — presets centralizados, conversão outline explícita eliminando heurística "size>0.05"
export type OutlineMode = 'none' | 'line' | 'shell';
export type LightingPreset = {
  id: string;
  label: string;
  intensity: number;
  shadow_softness: number;
  shadow_color: [number, number, number];
  light_dir: [number, number, number];
};
export type RenderPreset = LightingPreset & {
  outline: { mode: OutlineMode; width: number; color: [number, number, number]; opacity: number };
  material: { baseColor: string; roughness: number; metalness: number };
};
export const OUTLINE_PRESETS: Record<string, { mode: OutlineMode; width: number }> = {
  none: { mode: 'none', width: 0 },
  subtle: { mode: 'line', width: 0.008 },
  bold: { mode: 'line', width: 0.02 },
  shell: { mode: 'shell', width: 0.015 },
};
export function outlineFromPreset(id: string, fallback: OutlineMode = 'line'): { mode: OutlineMode; width: number } {
  return OUTLINE_PRESETS[id] ?? { mode: fallback, width: 0.012 };
}
export const LIGHTING_PRESETS: LightingPreset[] = [
  { id: 'soft', label: 'Suave', intensity: 0.9, shadow_softness: 0.5, shadow_color: [0.7, 0.72, 0.88], light_dir: [0.4, 0.9, 0.3] },
  { id: 'hard', label: 'Duro', intensity: 1.1, shadow_softness: 0.1, shadow_color: [0.55, 0.58, 0.78], light_dir: [0.6, 0.8, 0.2] },
  { id: 'studio', label: 'Estúdio', intensity: 1.0, shadow_softness: 0.3, shadow_color: [0.6, 0.62, 0.8], light_dir: [0.3, 1.0, 0.4] },
  { id: 'dramatic', label: 'Dramático', intensity: 1.2, shadow_softness: 0.15, shadow_color: [0.45, 0.48, 0.72], light_dir: [0.8, 0.6, 0.1] },
];
export function toLinearHex(hex: string): [number, number, number] {
  // shared sRGB→linear for presets (P1-01)
  const c = hex.trim().replace(/^#/, '');
  const full = c.length === 3 ? c.split('').map(ch=>ch+ch).join('') : c;
  const n = parseInt(full, 16);
  const R = ((n>>16)&255)/255, G=((n>>8)&255)/255, B=(n&255)/255;
  const toLin = (x:number)=> x<=0.04045 ? x/12.92 : Math.pow((x+0.055)/1.055, 2.4);
  return [toLin(R), toLin(G), toLin(B)];
}
