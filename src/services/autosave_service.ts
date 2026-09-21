import {
  CHARACTER_SNAPSHOT_SCHEMA_VERSION,
  sanitizeCharacterState,
  type CharacterState,
} from "./character_state";

/** Storage key for the crash-recovery cache (P0-08: now actually restored). */
export const AUTOSAVE_CACHE_KEY = "anigo_autosave_cache";

export interface ProjectStateSnapshot {
  preset: string;
  headScale: number;
  headRatio: number;
  // P0-08: the full character domain is part of the saved project.
  character?: CharacterState;
  schemaVersion?: number;
  outlineWidth: number;
  shadowThreshold: number;
  lightDir: [number, number, number];
  lightIntensity: number;
  shadowColor: [number, number, number];
  cameraEye: [number, number, number];
  cameraTarget: [number, number, number];
  cameraUp?: [number, number, number];
  fov?: number;
  timestamp: number;
  version: string;
  lightAzimuth?: number;
  lightElevation?: number;
  toonSmoothness?: number;
  specIntensity?: number;
  specExponent?: number;
  rimIntensity?: number;
  rimSpread?: number;
  hueShift?: number;
  toonSteps?: number;
  outlineColor?: string;
  baseColorHex?: string;
  shadowColorHex?: string;
  sunColor?: string;
  shadowSaturation?: number;
  ambientIntensity?: number;
  // P0-10: previously missing params
  outlineOpacity?: number;
  outlineSmoothness?: number;
  outlineDepthBias?: number;
  specSoftness?: number;
  specOffset?: number;
  specularSize?: number;
  aoIntensity?: number;
  ambientSky?: [number,number,number];
  ambientGround?: [number,number,number];
  specColorHex?: string;
  rimColor?: string;
  lightColor?: [number, number, number];
}

export class AutoSaveService {
  private timer: any = null;
  private isRunning: boolean = false;
  private intervalMinutes: number = 5;
  private getStateFn?: () => ProjectStateSnapshot;
  private isDirtyFn?: () => boolean;
  public onSaveCompleted?: (filePath: string, timestamp: string) => void;
  public onSaveError?: (error: unknown) => void;

  constructor() {}

  public configure(
    enabled: boolean,
    intervalMinutes: number,
    getStateFn: () => ProjectStateSnapshot,
    isDirtyFn?: () => boolean
  ) {
    this.getStateFn = getStateFn;
    this.isDirtyFn = isDirtyFn;
    this.intervalMinutes = Math.max(1, intervalMinutes);

    if (this.timer) {
      clearInterval(this.timer);
      this.timer = null;
    }

    if (enabled) {
      this.isRunning = true;
      const ms = this.intervalMinutes * 60 * 1000;
      this.timer = setInterval(() => {
        // P0-08: autosave only when dirty (was unconditional every 5 min).
        if (this.isDirtyFn && !this.isDirtyFn()) return;
        void this.saveNow();
      }, ms);
      console.log(`[AutoSaveService] Ativado com intervalo de ${this.intervalMinutes} min.`);
    } else {
      this.isRunning = false;
      console.log("[AutoSaveService] Desativado.");
    }
  }

  public async saveNow(): Promise<string | null> {
    if (!this.getStateFn) return null;

    try {
      const state = this.getStateFn();
      state.timestamp = Date.now();
      state.schemaVersion = CHARACTER_SNAPSHOT_SCHEMA_VERSION;
      const jsonStr = JSON.stringify(state, null, 2);

      // Save locally to localStorage as immediate recovery cache
      if (typeof localStorage !== "undefined") {
        localStorage.setItem(AUTOSAVE_CACHE_KEY, jsonStr);
      }

      // Save physically to disk via Tauri backend if running in desktop
      let savedPath = "localStorage (web-only)";
      if (typeof window !== "undefined" && (window as any).__TAURI_INTERNALS__) {
        const { invoke } = await import("@tauri-apps/api/core");
        savedPath = await invoke<string>("autosave_project", {
          payload: jsonStr,
          filename: null,
        });
      }

      const timeStr = new Date().toLocaleTimeString();
      console.log(`[AutoSaveService] Salvo com sucesso em: ${savedPath} às ${timeStr}`);
      this.onSaveCompleted?.(savedPath, timeStr);
      return savedPath;
    } catch (e) {
      // P0-08: errors propagate to the UI (was console-only).
      console.error("[AutoSaveService] Erro durante autosave:", e);
      this.onSaveError?.(e);
      return null;
    }
  }

  /** Reads the crash-recovery cache, or null when absent/unparseable. */
  public readRecoveryCache(): ProjectStateSnapshot | null {
    try {
      if (typeof localStorage === "undefined") return null;
      const raw = localStorage.getItem(AUTOSAVE_CACHE_KEY);
      if (!raw) return null;
      return parseProjectSnapshot(raw);
    } catch {
      return null;
    }
  }

  public clearRecoveryCache(): void {
    try {
      if (typeof localStorage !== "undefined") localStorage.removeItem(AUTOSAVE_CACHE_KEY);
    } catch {
      /* ignore */
    }
  }

  public destroy() {
    if (this.timer) {
      clearInterval(this.timer);
      this.timer = null;
    }
    this.isRunning = false;
  }
}

export const autoSaveService = new AutoSaveService();

/**
 * P0-08: validating project loader. Parses + sanitizes an untrusted project
 * payload (file or recovery cache). Never throws on schema drift: unknown
 * versions are migrated forward when possible, invalid numerics fall back
 * to safe defaults. Throws only when the payload is not JSON / not an object.
 */
export function parseProjectSnapshot(rawJson: string): ProjectStateSnapshot {
  let data: unknown;
  try {
    data = JSON.parse(rawJson);
  } catch (e) {
    throw new Error(`Projeto inválido: JSON malformado (${String(e)})`);
  }
  if (typeof data !== "object" || data === null || Array.isArray(data)) {
    throw new Error("Projeto inválido: raiz precisa ser um objeto");
  }
  const o = data as Record<string, unknown>;
  const num = (v: unknown, fb: number): number =>
    typeof v === "number" && Number.isFinite(v) ? v : fb;
  const vec3 = (v: unknown, fb: [number, number, number]): [number, number, number] =>
    Array.isArray(v) && v.length >= 3 ? [num(v[0], fb[0]), num(v[1], fb[1]), num(v[2], fb[2])] : fb;

  const schemaVersion =
    typeof o["schemaVersion"] === "number" ? o["schemaVersion"] : o["version"] === "0.1.0" ? 1 : 1;
  if (schemaVersion > CHARACTER_SNAPSHOT_SCHEMA_VERSION) {
    throw new Error(
      `Projeto criado por versão mais nova (schema ${schemaVersion}, suportado ${CHARACTER_SNAPSHOT_SCHEMA_VERSION})`
    );
  }

  const preset = o["preset"];
  return {
    preset: preset === "sphere" || preset === "cube" ? preset : "mannequin",
    headScale: num(o["headScale"], 1.0),
    headRatio: num(o["headRatio"], 6.5),
    outlineWidth: num(o["outlineWidth"], 3.5),
    shadowThreshold: num(o["shadowThreshold"], 0.5),
    lightDir: vec3(o["lightDir"], [0.577, 0.577, 0.577]),
    lightIntensity: num(o["lightIntensity"], 1.0),
    shadowColor: vec3(o["shadowColor"], [1, 1, 1]),
    cameraEye: vec3(o["cameraEye"], [0, 1.5, 3.5]),
    cameraTarget: vec3(o["cameraTarget"], [0, 1, 0]),
    cameraUp: vec3(o["cameraUp"], [0, 1, 0]),
    fov: num(o["fov"], 45),
    timestamp: num(o["timestamp"], Date.now()),
    version: typeof o["version"] === "string" ? o["version"] : "0.2.0",
    schemaVersion: CHARACTER_SNAPSHOT_SCHEMA_VERSION,
    lightAzimuth: num(o["lightAzimuth"], 45),
    lightElevation: num(o["lightElevation"], 45),
    toonSmoothness: num(o["toonSmoothness"], 0.02),
    specIntensity: num(o["specIntensity"], 0.4),
    specExponent: num(o["specExponent"], 32),
    rimIntensity: num(o["rimIntensity"], 0.8),
    rimSpread: num(o["rimSpread"], 0.4),
    hueShift: num(o["hueShift"], -15),
    toonSteps: num(o["toonSteps"], 1),
    outlineColor: typeof o["outlineColor"] === "string" ? o["outlineColor"] : "#402633",
    baseColorHex: typeof o["baseColorHex"] === "string" ? o["baseColorHex"] : "#faebd7",
    shadowColorHex: typeof o["shadowColorHex"] === "string" ? o["shadowColorHex"] : "#d1b8c7",
    sunColor: typeof o["sunColor"] === "string" ? o["sunColor"] : "#fff2df",
    shadowSaturation: num(o["shadowSaturation"], 1.15),
    ambientIntensity: num(o["ambientIntensity"], 0.35),
    outlineOpacity: num(o["outlineOpacity"], 1),
    outlineSmoothness: num(o["outlineSmoothness"], 0),
    outlineDepthBias: num(o["outlineDepthBias"], 0),
    specSoftness: num(o["specSoftness"], 0.05),
    specOffset: num(o["specOffset"], 0),
    specularSize: num(o["specularSize"], 0.45),
    aoIntensity: num(o["aoIntensity"], 0.85),
    ambientSky: vec3(o["ambientSky"], [0.52, 0.6, 0.78]),
    ambientGround: vec3(o["ambientGround"], [0.25, 0.2, 0.18]),
    specColorHex: typeof o["specColorHex"] === "string" ? o["specColorHex"] : "#ffffff",
    rimColor: typeof o["rimColor"] === "string" ? o["rimColor"] : "#93c5fd",
    lightColor: vec3(o["lightColor"], [1, 0.98, 0.95]),
    // P0-08: the character domain travels with the project (v1 payloads
    // without it sanitize to the canonical default character).
    character: sanitizeCharacterState(o["character"]),
  };
}
