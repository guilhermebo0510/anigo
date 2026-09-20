/**
 * ANIGO Studio - Professional Undo/Redo Engine
 * Adheres to ANIGO Inviolable Rules: Industrial-grade, zero amateur workarounds.
 * Implements Snapshot/Command pattern with slider coalescing and strict memory caps.
 */

export interface HistoryStateSnapshot {
  preset: "mannequin" | "sphere" | "cube";
  headScale: number;
  headRatio: number;
  outlineWidth: number;
  shadowThreshold: number;
  lightDir: [number, number, number];
  lightIntensity: number;
  shadowColor: [number, number, number];
  lightAzimuth?: number;
  lightElevation?: number;
  activeWorkspace?: string;
  activeTool?: string;
  projectName?: string;
  timestamp: number;
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
  // P0-10: previously missing params now persisted
  cameraEye?: [number, number, number];
  cameraTarget?: [number, number, number];
  cameraUp?: [number, number, number];
  fov?: number;
  outlineOpacity?: number;
  outlineSmoothness?: number;
  outlineDepthBias?: number;
  specSoftness?: number;
  specOffset?: number;
  specularSize?: number; // P2-07
  aoIntensity?: number; // P2-05
  ambientSky?: [number,number,number]; // P2-04
  ambientGround?: [number,number,number];
  specColorHex?: string;
  rimColor?: string;
  lightColor?: [number, number, number];
}

export interface HistoryEntry {
  snapshot: HistoryStateSnapshot;
  description: string;
  timestamp: number;
}

class HistoryService {
  private undoStack: HistoryEntry[] = [];
  private redoStack: HistoryEntry[] = [];
  private currentSnapshot: HistoryStateSnapshot | null = null;
  private readonly MAX_HISTORY = 60;
  private lastActionTime = 0;
  private lastActionType = "";
  public isExecutingHistory = false;

  public onStackChange?: (canUndo: boolean, canRedo: boolean, lastAction?: string) => void;

  /**
   * Initializes or resets the history baseline (e.g., at app startup or project load).
   */
  public init(initialSnapshot: HistoryStateSnapshot): void {
    this.undoStack = [];
    this.redoStack = [];
    this.currentSnapshot = JSON.parse(JSON.stringify(initialSnapshot));
    this.lastActionTime = 0;
    this.lastActionType = "";
    this.notifyChange();
  }

  /**
   * Pushes a new state.
   * The snapshot prior to mutation is preserved in the undo stack.
   * If `isContinuous` is true, continuous updates (e.g., dragging sliders)
   * within 500ms coalesce so that the baseline before the drag is preserved.
   */
  public push(newSnapshot: HistoryStateSnapshot, description: string, isContinuous = false): void {
    if (this.isExecutingHistory) return;

    const now = Date.now();
    const deepClone: HistoryStateSnapshot = JSON.parse(JSON.stringify(newSnapshot));

    if (!this.currentSnapshot) {
      this.currentSnapshot = deepClone;
      return;
    }

    if (
      isContinuous &&
      this.undoStack.length > 0 &&
      this.lastActionType === description &&
      now - this.lastActionTime < 500
    ) {
      // Coalescing continuous adjustments (e.g. slider drags):
      // The undoStack already contains the snapshot before the drag started.
      // We just update the current state!
      this.currentSnapshot = deepClone;
    } else {
      // Normal action:
      // Push the state BEFORE the mutation onto the undo stack!
      this.undoStack.push({
        snapshot: this.currentSnapshot,
        description,
        timestamp: now,
      });

      if (this.undoStack.length > this.MAX_HISTORY) {
        this.undoStack.shift();
      }

      this.currentSnapshot = deepClone;
    }

    // Any new user action invalidates the redo stack
    this.redoStack = [];
    this.lastActionTime = now;
    this.lastActionType = description;
    this.notifyChange();
  }

  /**
   * Undoes the last action and returns the restored snapshot.
   */
  public undo(): HistoryStateSnapshot | null {
    if (this.undoStack.length === 0 || !this.currentSnapshot) return null;

    this.isExecutingHistory = true;
    const previousEntry = this.undoStack.pop()!;

    // Push current snapshot onto redo stack
    this.redoStack.push({
      snapshot: this.currentSnapshot,
      description: previousEntry.description,
      timestamp: Date.now(),
    });

    this.currentSnapshot = JSON.parse(JSON.stringify(previousEntry.snapshot));
    this.isExecutingHistory = false;
    this.notifyChange();

    return previousEntry.snapshot;
  }

  /**
   * Redoes the last undone action and returns the restored snapshot.
   */
  public redo(): HistoryStateSnapshot | null {
    if (this.redoStack.length === 0 || !this.currentSnapshot) return null;

    this.isExecutingHistory = true;
    const nextEntry = this.redoStack.pop()!;

    // Push current snapshot onto undo stack
    this.undoStack.push({
      snapshot: this.currentSnapshot,
      description: nextEntry.description,
      timestamp: Date.now(),
    });

    this.currentSnapshot = JSON.parse(JSON.stringify(nextEntry.snapshot));
    this.isExecutingHistory = false;
    this.notifyChange();

    return nextEntry.snapshot;
  }

  public canUndo(): boolean {
    return this.undoStack.length > 0;
  }

  public canRedo(): boolean {
    return this.redoStack.length > 0;
  }

  public getLastUndoDescription(): string | undefined {
    return this.undoStack[this.undoStack.length - 1]?.description;
  }

  public getLastRedoDescription(): string | undefined {
    return this.redoStack[this.redoStack.length - 1]?.description;
  }

  public clear(): void {
    this.undoStack = [];
    this.redoStack = [];
    this.currentSnapshot = null;
    this.notifyChange();
  }

  private notifyChange(): void {
    if (this.onStackChange) {
      this.onStackChange(
        this.canUndo(),
        this.canRedo(),
        this.getLastUndoDescription()
      );
    }
  }
}

export const historyService = new HistoryService();
