// P1-07/P1-08 bridge normalization — single consumer validates & clamps
export type Normalized =
  | { kind: 'camera_orbit' | 'camera_zoom' | 'camera_pan'; [k:string]: any }
  | { kind: 'set_light'; dir: [number,number,number]; intensity: number; shadow_softness: number; shadow_color: [number,number,number] }
  | { kind: 'set_outline'; mode: string; width: number; color: [number,number,number]; opacity: number }
  | { kind: 'set_material'; baseColor: string; roughness: number; metalness: number };
const clamp = (v:number,a:number,b:number)=> Math.max(a, Math.min(b, v));
const isFiniteNum = (v:any)=> typeof v==='number' && Number.isFinite(v);
export function normalizeBridgePayload(event: string, raw: any): any | null {
  if (raw == null || typeof raw !== 'object') return null;
  switch(event){
    case 'anigo://camera_orbit': {
      if (!isFiniteNum(raw.deltaYaw) || !isFiniteNum(raw.deltaPitch)) return null;
      return { kind:'camera_orbit', deltaYaw: clamp(raw.deltaYaw,-10,10), deltaPitch: clamp(raw.deltaPitch,-10,10) };
    }
    case 'anigo://camera_zoom': {
      if (!isFiniteNum(raw.factor)) return null;
      return { kind:'camera_zoom', factor: clamp(raw.factor, 0.1, 10) };
    }
    case 'anigo://set_light': {
      const dir = Array.isArray(raw.light_dir) ? raw.light_dir : raw.dir;
      if (!Array.isArray(dir) || dir.length!==3 || !dir.every(isFiniteNum)) return null;
      const len = Math.hypot(dir[0],dir[1],dir[2]) || 1;
      const n: [number,number,number] = [dir[0]/len, dir[1]/len, dir[2]/len];
      return { kind:'set_light', dir: n, intensity: isFiniteNum(raw.intensity)?clamp(raw.intensity,0,4):1, shadow_softness: isFiniteNum(raw.shadow_softness)?clamp(raw.shadow_softness,0,1):0.3, shadow_color: Array.isArray(raw.shadow_color)&&raw.shadow_color.length===3?raw.shadow_color:[1,1,1] };
    }
    case 'anigo://set_outline': {
      const w = isFiniteNum(raw.width)?clamp(raw.width,0,0.05):0.012;
      return { kind:'set_outline', mode: String(raw.mode||'line'), width: w, color: raw.color, opacity: isFiniteNum(raw.opacity)?clamp(raw.opacity,0,1):1 };
    }
    case 'anigo://set_material': {
      return { kind:'set_material', baseColor: typeof raw.baseColor==='string'?raw.baseColor:'#ffffff', roughness: isFiniteNum(raw.roughness)?clamp(raw.roughness,0,1):0.5, metalness: isFiniteNum(raw.metalness)?clamp(raw.metalness,0,1):0 };
    }
    default: return raw;
  }
}
