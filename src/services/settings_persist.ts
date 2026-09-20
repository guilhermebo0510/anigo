// P1-11 settings persistence — localStorage with versioned key, handles DPI/system caps
export type PersistedSettings = {
  version: 1;
  targetFps: number;
  useWebGpu: boolean;
  enableMsaa: boolean;
  stylePreset: string;
  locale: string;
  dpiAware: boolean;
};
const KEY = 'anigo:settings:v1';
const DEFAULTS: PersistedSettings = {
  version: 1,
  targetFps: 60,
  useWebGpu: true,
  enableMsaa: true,
  stylePreset: 'default',
  locale: 'pt-BR',
  dpiAware: true,
};
export function loadSettings(): PersistedSettings {
  try {
    const raw = localStorage.getItem(KEY);
    if (!raw) return { ...DEFAULTS };
    const parsed = JSON.parse(raw);
    return { ...DEFAULTS, ...parsed, version: 1 };
  } catch { return { ...DEFAULTS }; }
}
export function saveSettings(patch: Partial<PersistedSettings>): void {
  try {
    const cur = loadSettings();
    const next = { ...cur, ...patch, version: 1 as const };
    localStorage.setItem(KEY, JSON.stringify(next));
  } catch {}
}
export function devicePixelRatioSafe(): number {
  // P1-11 clamp DPR to avoid 3x mobile memory explosion
  const raw = (typeof window !== 'undefined' ? window.devicePixelRatio ?? window.devicePixelRatio : 1) as number;
  const v = Number.isFinite(raw) ? raw : 1;
  return Math.min(Math.max(v, 1), 2);
}
// alias typo compat
declare global { interface Window { devicePixelRatio?: number; } }
