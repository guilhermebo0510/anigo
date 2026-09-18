// ANIGO Canonical Character Factory Presets (Sub-Sprint 3.14)
// High-grade morphology configurations combining somatotype, BOND proportions and morph sliders.

export interface CharacterPreset {
  id: string;
  name: string;
  description: string;
  model: "male" | "female";
  somatotype: {
    endo: number;
    meso: number;
    ecto: number;
  };
  genderDimorphism: number;
  proportions: {
    headScale: number;
    headRatio: number;
    shoulderWidth: number;
    legLength: number;
    armLength: number;
    neckLength: number;
    torsoLength?: number;
    heightOverall?: number;
  };
  sliders: Record<string, number>;
}

export const CANONICAL_CHARACTER_PRESETS: CharacterPreset[] = [
  {
    id: "shonen_hero",
    name: "Shonen Hero",
    description: "Proporção heróica 6.5c atlética com V-taper definido e queixo V-Line estilizado.",
    model: "male",
    somatotype: { endo: 0.15, meso: 0.55, ecto: 0.30 },
    genderDimorphism: 1.0,
    proportions: {
      headScale: 1.00,
      headRatio: 6.5,
      shoulderWidth: 1.15,
      legLength: 1.05,
      armLength: 1.02,
      neckLength: 1.00,
    },
    sliders: {
      jaw_v_line_taper: 0.70,
      brow_ridge_prominence: 0.30,
      abs_sixpack_definition: 0.50,
      pectoral_muscle_bulk: 0.40,
      eye_canthal_tilt: 5.0,
      deltoid_muscle_volume: 0.45,
      latissimus_dorsi_flare: 0.40,
      thigh_circumference: 1.05,
    },
  },
  {
    id: "shojo_idol",
    name: "Shojo Idol",
    description: "Silhueta ampulheta delicada 6.2c com olhos expressivos tareme e cintura afilada.",
    model: "female",
    somatotype: { endo: 0.30, meso: 0.15, ecto: 0.55 },
    genderDimorphism: 0.0,
    proportions: {
      headScale: 1.05,
      headRatio: 6.2,
      shoulderWidth: 0.90,
      legLength: 1.10,
      armLength: 0.98,
      neckLength: 1.05,
    },
    sliders: {
      bust_volume_cup: 0.45,
      waist_pinch_width: 0.85,
      hip_trochanteric_flare: 1.20,
      eye_scale_uniform: 1.25,
      eye_canthal_tilt: -6.0,
      jaw_v_line_taper: 0.85,
      cheek_fullness_upper: 0.40,
      inner_thigh_gap: 0.30,
    },
  },
  {
    id: "muscular_berserker",
    name: "Muscular Berserker",
    description: "Físico hipertrofiado de 7.5c com musculatura densa, trapézio maciço e mandíbula potente.",
    model: "male",
    somatotype: { endo: 0.10, meso: 0.85, ecto: 0.05 },
    genderDimorphism: 1.0,
    proportions: {
      headScale: 0.90,
      headRatio: 7.5,
      shoulderWidth: 1.35,
      legLength: 1.05,
      armLength: 1.10,
      neckLength: 0.95,
    },
    sliders: {
      deltoid_muscle_volume: 0.90,
      biceps_peak_volume: 0.85,
      triceps_bulk: 0.80,
      pectoral_muscle_bulk: 0.90,
      trapezius_bulk: 0.85,
      abs_sixpack_definition: 0.90,
      jaw_bigonial_width: 1.25,
      neck_circumference: 1.35,
      thigh_circumference: 1.30,
      calf_circumference: 1.25,
    },
  },
  {
    id: "plus_size",
    name: "Plus Size / Chubby",
    description: "Morfologia curvilínea e encorpada com quadril proeminente e silhueta volumosa suave.",
    model: "female",
    somatotype: { endo: 0.75, meso: 0.20, ecto: 0.05 },
    genderDimorphism: 0.0,
    proportions: {
      headScale: 1.02,
      headRatio: 6.0,
      shoulderWidth: 1.05,
      legLength: 0.98,
      armLength: 0.98,
      neckLength: 0.95,
    },
    sliders: {
      belly_visceral_protuberance: 0.65,
      flank_love_handles: 0.60,
      cheek_fullness_upper: 0.75,
      thigh_circumference: 1.35,
      hip_trochanteric_flare: 1.30,
      bust_volume_cup: 1.10,
      gluteus_volume_overall: 1.35,
      submental_fullness: 0.40,
    },
  },
  {
    id: "chibi",
    name: "Chibi 2.5c",
    description: "Super-deformed canônico de 2.5 cabeças com crânio proeminente, olhos gigantes e traços fofos.",
    model: "female",
    somatotype: { endo: 0.45, meso: 0.10, ecto: 0.45 },
    genderDimorphism: 0.5,
    proportions: {
      headScale: 1.55,
      headRatio: 2.5,
      shoulderWidth: 0.75,
      legLength: 0.75,
      armLength: 0.75,
      neckLength: 0.75,
    },
    sliders: {
      eye_scale_uniform: 1.55,
      cheek_fullness_upper: 0.95,
      jaw_v_line_taper: 0.75,
      chin_length: 0.80,
      nose_tip_sharpness: 0.10,
      ear_scale_uniform: 1.20,
    },
  },
  {
    id: "heroic_8_5c",
    name: "Stylized Heroic 8.5c",
    description: "Proporção fashion/JoJo heróica com pernas ultra-longas, ombros largos e porte esguio imponente.",
    model: "male",
    somatotype: { endo: 0.05, meso: 0.60, ecto: 0.35 },
    genderDimorphism: 1.0,
    proportions: {
      headScale: 0.80,
      headRatio: 8.5,
      shoulderWidth: 1.25,
      legLength: 1.35,
      armLength: 1.20,
      neckLength: 1.15,
    },
    sliders: {
      jaw_v_line_taper: 0.85,
      deltoid_muscle_volume: 0.65,
      latissimus_dorsi_flare: 0.70,
      outer_thigh_sweep: 0.65,
      quadriceps_definition: 0.60,
      clavicle_bone_relief: 0.75,
      eye_canthal_tilt: 10.0,
    },
  },
];

export function getCharacterPreset(id: string): CharacterPreset | undefined {
  return CANONICAL_CHARACTER_PRESETS.find((p) => p.id === id);
}
