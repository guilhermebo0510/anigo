/**
 * ANIGO — Câmera Cinematográfica (Fase 2 #53)
 *
 * Referência TS da matemática cinematográfica: presets de lente (focal ↔
 * FOV), reframe sem salta de orbitador, Circle of Confusion / raio de
 * bokeh e tracking de alvo com amortecimento exponencial.
 *
 * Esta é a MESMA conta que o núcleo Rust faz em `anigo-core::math`
 * (lens_fov_y / reframe_radius / circle_of_confusion / damp_value) e que o
 * shader `postprocess_dof.wgsl` repete no GPU (CoC idêntica nos três). Os
 * valores dourados vivem no contrato, seção `cinematography` — este módulo
 * os reproduz (testes/contracts/cinematography.test.ts).
 */

// ─── Presets de lente (sensor full-frame, altura 24 mm) ────────────────────

/** Altura do sensor full-frame em mm — o mesmo de `math::SENSOR_HEIGHT_MM`. */
export const SENSOR_HEIGHT_MM = 24.0;

export interface LensPreset {
  id: string;
  focalMm: number;
  name: string;
}

/** Presets cinematográficos do issue (24 ação / 35 corpo / 50 humano / 85 retrato / 135 close-up). */
export const LENS_PRESETS: LensPreset[] = [
  { id: "24", focalMm: 24.0, name: "Ação Panorâmica" },
  { id: "35", focalMm: 35.0, name: "Corpo Inteiro" },
  { id: "50", focalMm: 50.0, name: "Visão Humana Neutra" },
  { id: "85", focalMm: 85.0, name: "Retrato Anime" },
  { id: "135", focalMm: 135.0, name: "Close-up Dramático" },
];

/** FOV vertical (radianos) de uma distância focal em mm: fov_y = 2·atan(sensor/2 / focal). */
export function lensFovY(focalMm: number, sensorHeightMm = SENSOR_HEIGHT_MM): number {
  return 2 * Math.atan((sensorHeightMm * 0.5) / focalMm);
}

/**
 * Raio do orbitador que mantém o enquadramento do sujeito ao trocar de
 * lente (aceite 2: 24 → 85 mm muda a perspectiva sem mover o orbitador
 * bruscamente): r' = r · tan(fovFrom/2) / tan(fovTo/2).
 */
export function reframeRadius(radius: number, fovFrom: number, fovTo: number): number {
  return (radius * Math.tan(fovFrom * 0.5)) / Math.tan(fovTo * 0.5);
}

// ─── Depth of Field (Circle of Confusion) ───────────────────────────────────

/**
 * Circle of Confusion (m, no sensor) de um ponto a `fragDist` com foco em
 * `focusDist` — a fórmula do issue:
 *   CoC = |(D − F_dist) / D| × F² / (N × (F_dist − F))
 * D = distância do fragmento, F_dist = distância de foco, F = focal (m), N = f.
 */
export function circleOfConfusion(
  fragDist: number,
  focusDist: number,
  focalM: number,
  fNumber: number
): number {
  const d = Math.max(fragDist, 1e-4);
  const fd = Math.max(focusDist, 1e-4);
  const f = Math.max(focalM, 1e-4);
  const n = Math.max(fNumber, 0.05);
  return (Math.abs(fd - d) / d) * (f * f) / (n * Math.max(fd - f, 1e-4));
}

/**
 * Raio do bokeh em pixels: CoC (m no sensor) → fração da altura do sensor →
 * px da imagem, limitado por `maxRadiusPx` (teto de custo).
 */
export function bokehRadiusPx(
  cocM: number,
  imageHeightPx: number,
  sensorHeightM = SENSOR_HEIGHT_MM * 0.001,
  maxRadiusPx = Infinity
): number {
  return Math.min((cocM / sensorHeightM) * imageHeightPx, maxRadiusPx);
}

/**
 * Amostragem da profundidade de hardware (zero_to_one, RH) → distância de
 * vista em metros: z = near·far / (far − d·(far − near)). Idêntico a
 * `linearize_depth` no postprocess_dof.wgsl.
 */
export function linearizeDepth(d: number, zNear: number, zFar: number): number {
  return (zNear * zFar) / (zFar - d * (zFar - zNear));
}

// ─── Amortecimento (damping) ────────────────────────────────────────────────

/**
 * Suavização exponencial (criticamente amortecida):
 * x += (target − x) × (1 − e^(−damping × dt)). damping em 1/s; nunca
 * ultrapassa o alvo; damping 0 ou dt 0 deixam o valor imutável.
 */
export function dampValue(current: number, target: number, dampingPerSecond: number, deltaSeconds: number): number {
  const k = 1 - Math.exp(-Math.max(dampingPerSecond, 0) * Math.max(deltaSeconds, 0));
  return current + (target - current) * k;
}

export type Vec3 = [number, number, number];

/** Suavização exponencial de um vetor (componente a componente). */
export function dampVec3(
  current: Vec3,
  target: Vec3,
  dampingPerSecond: number,
  deltaSeconds: number
): Vec3 {
  return [
    dampValue(current[0], target[0], dampingPerSecond, deltaSeconds),
    dampValue(current[1], target[1], dampingPerSecond, deltaSeconds),
    dampValue(current[2], target[2], dampingPerSecond, deltaSeconds),
  ];
}

// ─── Tracking de alvo (CameraTarget) ────────────────────────────────────────

export type TrackingMode = "off" | "head" | "hips" | "poi";

/** Damping padrão do tracking (1/s) — o mesmo do contrato. */
export const TRACKING_DAMPING_DEFAULT = 6.0;

/**
 * Pontos canônicos do esqueleto humanoide para travar o foco: cabeça
 * (close-up de rosto — onde os olhos ficam) e centro de massa (Hips).
 */
export const TRACKING_TARGET_POSITIONS: Record<"head" | "hips", Vec3> = {
  head: [0.0, 1.49, 0.0],
  hips: [0.0, 0.85, 0.0],
};

/** Desejo atual do alvo (posição onde a câmera deve mirar) para o modo ativo. */
export function trackingDesiredTarget(mode: TrackingMode, poi: Vec3): Vec3 {
  switch (mode) {
    case "head":
      return TRACKING_TARGET_POSITIONS.head;
    case "hips":
      return TRACKING_TARGET_POSITIONS.hips;
    case "poi":
      return poi;
    case "off":
      return [0, 0, 0];
  }
}

/**
 * Um passo do tracking (roda a cada frame no viewport): o alvo da câmera
 * persegue a posição desejada com amortecimento exponencial e o olho move
 * junto com o MESMO delta (translação rígida do rig) — o enquadramento é
 * mantido enquanto o personagem se move (aceite 3). Modo "off" não toca
 * nada; o orbitador livre continua funcionando por cima.
 */
export function stepTracking(
  target: Vec3,
  eye: Vec3,
  mode: TrackingMode,
  dampingPerSecond: number,
  poi: Vec3,
  deltaSeconds: number
): { target: Vec3; eye: Vec3; moved: boolean } {
  if (mode === "off" || deltaSeconds <= 0) {
    return { target, eye, moved: false };
  }
  const desired = trackingDesiredTarget(mode, poi);
  const nextTarget = dampVec3(target, desired, dampingPerSecond, deltaSeconds);
  const nextEye: Vec3 = [
    eye[0] + (nextTarget[0] - target[0]),
    eye[1] + (nextTarget[1] - target[1]),
    eye[2] + (nextTarget[2] - target[2]),
  ];
  const moved =
    Math.abs(nextTarget[0] - target[0]) +
      Math.abs(nextTarget[1] - target[1]) +
      Math.abs(nextTarget[2] - target[2]) >
    1e-9;
  return { target: nextTarget, eye: nextEye, moved };
}

// ─── DoF settings (mesma forma de Scene.dof / DofSettings do núcleo) ────────

export interface DofSettings {
  enabled: boolean;
  /** Distância de foco (m) — o plano milimetricamente nítido. */
  focusDistance: number;
  /** Número f (abertura) — menor = mais bokeh. */
  fNumber: number;
  /** Distância focal ativa em mm. */
  focalMm: number;
  /** 0 = bokeh circular, 1 = hexagonal. */
  bokehShape: 0 | 1;
  /** Raio máximo do bokeh em px. */
  maxRadiusPx: number;
}

/** Defaults do contrato (cinematography.dof.defaults) — DoF desligado. */
export const DEFAULT_DOF_SETTINGS: DofSettings = {
  enabled: false,
  focusDistance: 2.0,
  fNumber: 2.0,
  focalMm: 50.0,
  bokehShape: 0,
  maxRadiusPx: 16.0,
};
