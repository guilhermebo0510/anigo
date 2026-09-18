export interface ProjectStateSnapshot {
  preset: string;
  headScale: number;
  headRatio: number;
  outlineWidth: number;
  shadowThreshold: number;
  lightDir: [number, number, number];
  lightIntensity: number;
  shadowColor: [number, number, number];
  cameraEye: [number, number, number];
  cameraTarget: [number, number, number];
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
}

export class AutoSaveService {
  private timer: any = null;
  private isRunning: boolean = false;
  private intervalMinutes: number = 5;
  private getStateFn?: () => ProjectStateSnapshot;
  public onSaveCompleted?: (filePath: string, timestamp: string) => void;

  constructor() {}

  public configure(
    enabled: boolean,
    intervalMinutes: number,
    getStateFn: () => ProjectStateSnapshot
  ) {
    this.getStateFn = getStateFn;
    this.intervalMinutes = Math.max(1, intervalMinutes);

    if (this.timer) {
      clearInterval(this.timer);
      this.timer = null;
    }

    if (enabled) {
      this.isRunning = true;
      const ms = this.intervalMinutes * 60 * 1000;
      this.timer = setInterval(() => {
        this.saveNow();
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
      const jsonStr = JSON.stringify(state, null, 2);

      // Save locally to localStorage as immediate recovery cache
      if (typeof localStorage !== "undefined") {
        localStorage.setItem("anigo_autosave_cache", jsonStr);
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
      console.error("[AutoSaveService] Erro durante autosave:", e);
      return null;
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
