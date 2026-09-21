import {
  CHARACTER_SNAPSHOT_SCHEMA_VERSION,
  type CharacterState,
} from "./character_state";
import {
  DEFAULT_APP_VERSION,
  createEnvelope,
  parseEnvelope,
  readRecovery,
  serializeEnvelope,
  toProjectSnapshot,
  toSessionState,
  type ProjectEnvelopeV3,
  type RecoveryResult,
  type SessionState,
} from "./project_persistence";
import type { CanonicalProjectDocumentV1 } from "../contracts/project_state.v1";
import type { CommandLogWire } from "../contracts/command_log.v1";

/** Storage key for the crash-recovery cache (P0-08: now actually restored). */
export const AUTOSAVE_CACHE_KEY = "anigo_autosave_cache";

export interface ProjectStateSnapshot {
  /** v3: the scene domain travels inside the session block. */
  scene?: unknown;
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

export interface AutoSaveOptions {
  /** Canonical project document authored by the Rust core (null in preview). */
  getCoreProject?: () => CanonicalProjectDocumentV1 | null;
  /** Opaque UI preferences persisted with the session. */
  getUiState?: () => Record<string, unknown>;
  /**
   * P0 undo/redo: accepted commands of the session. Persisted inside the session
   * block so the whole session can be rebuilt by replaying it (base + log).
   */
  getCommandLog?: () => CommandLogWire | null;
  appVersion?: string;
}

export class AutoSaveService {
  private timer: any = null;
  private isRunning: boolean = false;
  private intervalMinutes: number = 5;
  private getStateFn?: () => ProjectStateSnapshot;
  private isDirtyFn?: () => boolean;
  private options: AutoSaveOptions = {};
  public onSaveCompleted?: (filePath: string, timestamp: string) => void;
  public onSaveError?: (error: unknown) => void;

  constructor() {}

  public configure(
    enabled: boolean,
    intervalMinutes: number,
    getStateFn: () => ProjectStateSnapshot,
    isDirtyFn?: () => boolean,
    options: AutoSaveOptions = {}
  ) {
    this.getStateFn = getStateFn;
    this.isDirtyFn = isDirtyFn;
    this.options = options;
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
      // P0 persistence: everything written to disk is a versioned envelope.
      const envelope = this.buildEnvelope(state);
      const jsonStr = serializeEnvelope(envelope);

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

  /** Builds the envelope for the current session (never throws on core errors). */
  public buildEnvelope(state: ProjectStateSnapshot): ProjectEnvelopeV3 {
    let coreProject: CanonicalProjectDocumentV1 | null = null;
    if (this.options.getCoreProject) {
      try {
        coreProject = this.options.getCoreProject() ?? null;
      } catch (error) {
        // A core that cannot answer must not break the autosave: the envelope
        // records the degraded `preview` source instead of losing the session.
        console.warn("[AutoSaveService] núcleo não forneceu o documento canônico:", error);
        coreProject = null;
      }
    }
    const session = toSessionState(state);
    const commandLog = this.options.getCommandLog?.() ?? null;
    if (commandLog && commandLog.entries.length > 0) {
      // Only commands that the history accepted ever reach this point.
      session.command_log = commandLog;
    }
    return createEnvelope({
      session,
      coreProject,
      ui: this.options.getUiState?.() ?? {},
      savedAt: state.timestamp ?? Date.now(),
      appVersion: this.options.appVersion ?? state.version ?? DEFAULT_APP_VERSION,
    });
  }

  /**
   * Reads the crash-recovery cache: migrates legacy payloads, validates the
   * envelope and reports *why* a payload was rejected.
   */
  public recoverSession(): RecoveryResult {
    let raw: string | null = null;
    try {
      if (typeof localStorage !== "undefined") raw = localStorage.getItem(AUTOSAVE_CACHE_KEY);
    } catch {
      raw = null;
    }
    return readRecovery(raw);
  }

  /** Reads the crash-recovery cache as the flat project snapshot, or null. */
  public readRecoveryCache(): ProjectStateSnapshot | null {
    const result = this.recoverSession();
    return result.envelope ? toProjectSnapshot(result.envelope.session) : null;
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
 * P0 persistence: validating project loader for files and recovery caches.
 *
 * Accepts every format the app has ever written (v1 flat, v2 flat, v3
 * envelope), migrates it forward with explicit steps and validates the result
 * before returning. Invalid numerics are sanitized; a payload that cannot be
 * trusted (malformed JSON, non-object root, newer schema) throws a typed
 * `PersistenceError` the caller can report to the user.
 */
export function parseProjectSnapshot(rawJson: string): ProjectStateSnapshot {
  return toProjectSnapshot(parseProjectDocument(rawJson).session);
}

/** Full read of a persisted project, including the canonical core document. */
export function parseProjectDocument(rawJson: string): {
  envelope: ProjectEnvelopeV3;
  session: SessionState;
  migrations: string[];
} {
  const { envelope, applied } = parseEnvelope(rawJson);
  return { envelope, session: envelope.session, migrations: applied };
}
