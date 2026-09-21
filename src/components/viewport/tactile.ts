// ANIGO Viewport 3D Tactile Manipulation Engine (Sub-Sprint 3.13)
// Real-time raycasting against canonical humanoid collision capsules and screen-to-slider projection.

import { isKnownSliderId, type SliderId } from "../../services/character_state";

export type AnatomicalSegment =
  | "Head"
  | "Neck"
  | "Chest"
  | "Waist"
  | "Pelvis"
  | "LeftUpperArm"
  | "RightUpperArm"
  | "LeftForearm"
  | "RightForearm"
  | "LeftThigh"
  | "RightThigh"
  | "LeftCalf"
  | "RightCalf";

export interface Ray {
  origin: [number, number, number];
  direction: [number, number, number];
}

export interface RaycastHit {
  segment: AnatomicalSegment;
  distance: number;
  hitPoint: [number, number, number];
  normal: [number, number, number];
}

export interface Capsule {
  segment: AnatomicalSegment;
  start: [number, number, number];
  end: [number, number, number];
  radius: number;
}

export interface TactileDragResult {
  primarySlider: SliderId;
  primaryDelta: number;
  secondarySlider?: SliderId;
  secondaryDelta?: number;
}

export const CANONICAL_HUMANOID_CAPSULES: Capsule[] = [
  // 1. Head
  {
    segment: "Head",
    start: [0.0, 1.55, 0.0],
    end: [0.0, 1.68, 0.0],
    radius: 0.105,
  },
  // 2. Neck
  {
    segment: "Neck",
    start: [0.0, 1.38, 0.0],
    end: [0.0, 1.48, 0.0],
    radius: 0.055,
  },
  // 3. Chest
  {
    segment: "Chest",
    start: [0.0, 1.16, 0.0],
    end: [0.0, 1.36, 0.0],
    radius: 0.155,
  },
  // 4. Waist
  {
    segment: "Waist",
    start: [0.0, 0.96, 0.0],
    end: [0.0, 1.14, 0.0],
    radius: 0.125,
  },
  // 5. Pelvis
  {
    segment: "Pelvis",
    start: [0.0, 0.77, 0.0],
    end: [0.0, 0.92, 0.0],
    radius: 0.150,
  },
  // 6. Arms
  {
    segment: "LeftUpperArm",
    start: [0.18, 1.36, 0.0],
    end: [0.24, 1.12, 0.0],
    radius: 0.055,
  },
  {
    segment: "RightUpperArm",
    start: [-0.18, 1.36, 0.0],
    end: [-0.24, 1.12, 0.0],
    radius: 0.055,
  },
  {
    segment: "LeftForearm",
    start: [0.24, 1.12, 0.0],
    end: [0.28, 0.88, 0.0],
    radius: 0.048,
  },
  {
    segment: "RightForearm",
    start: [-0.24, 1.12, 0.0],
    end: [-0.28, 0.88, 0.0],
    radius: 0.048,
  },
  // 7. Legs
  {
    segment: "LeftThigh",
    start: [0.09, 0.76, 0.0],
    end: [0.09, 0.44, 0.0],
    radius: 0.075,
  },
  {
    segment: "RightThigh",
    start: [-0.09, 0.76, 0.0],
    end: [-0.09, 0.44, 0.0],
    radius: 0.075,
  },
  {
    segment: "LeftCalf",
    start: [0.09, 0.44, 0.0],
    end: [0.09, 0.12, 0.0],
    radius: 0.058,
  },
  {
    segment: "RightCalf",
    start: [-0.09, 0.44, 0.0],
    end: [-0.09, 0.12, 0.0],
    radius: 0.058,
  },
];

function intersectSphere(ray: Ray, center: [number, number, number], radius: number): { t: number; normal: [number, number, number] } | null {
  const oc: [number, number, number] = [
    ray.origin[0] - center[0],
    ray.origin[1] - center[1],
    ray.origin[2] - center[2],
  ];
  const b = oc[0] * ray.direction[0] + oc[1] * ray.direction[1] + oc[2] * ray.direction[2];
  const c = oc[0] * oc[0] + oc[1] * oc[1] + oc[2] * oc[2] - radius * radius;
  const discr = b * b - c;

  if (discr < 0) return null;

  const sqrtDiscr = Math.sqrt(discr);
  const t0 = -b - sqrtDiscr;
  const t1 = -b + sqrtDiscr;

  let t = -1;
  if (t0 > 1e-5) {
    t = t0;
  } else if (t1 > 1e-5) {
    t = t1;
  } else {
    return null;
  }

  const p: [number, number, number] = [
    ray.origin[0] + ray.direction[0] * t,
    ray.origin[1] + ray.direction[1] * t,
    ray.origin[2] + ray.direction[2] * t,
  ];
  const nLen = Math.hypot(p[0] - center[0], p[1] - center[1], p[2] - center[2]);
  const normal: [number, number, number] = nLen > 1e-6
    ? [(p[0] - center[0]) / nLen, (p[1] - center[1]) / nLen, (p[2] - center[2]) / nLen]
    : [0, 1, 0];

  return { t, normal };
}

export function intersectCapsule(ray: Ray, capsule: Capsule): RaycastHit | null {
  let minT = Infinity;
  let hitNorm: [number, number, number] = [0, 1, 0];

  // 1. Sphere at start
  const sStart = intersectSphere(ray, capsule.start, capsule.radius);
  if (sStart && sStart.t < minT) {
    minT = sStart.t;
    hitNorm = sStart.normal;
  }

  // 2. Sphere at end
  const sEnd = intersectSphere(ray, capsule.end, capsule.radius);
  if (sEnd && sEnd.t < minT) {
    minT = sEnd.t;
    hitNorm = sEnd.normal;
  }

  // 3. Cylinder between start and end
  const ab: [number, number, number] = [
    capsule.end[0] - capsule.start[0],
    capsule.end[1] - capsule.start[1],
    capsule.end[2] - capsule.start[2],
  ];
  const abLen = Math.hypot(ab[0], ab[1], ab[2]);
  if (abLen > 1e-5) {
    const abDir: [number, number, number] = [ab[0] / abLen, ab[1] / abLen, ab[2] / abLen];
    const ao: [number, number, number] = [
      ray.origin[0] - capsule.start[0],
      ray.origin[1] - capsule.start[1],
      ray.origin[2] - capsule.start[2],
    ];

    const dDotAb = ray.direction[0] * abDir[0] + ray.direction[1] * abDir[1] + ray.direction[2] * abDir[2];
    const vPerp: [number, number, number] = [
      ray.direction[0] - abDir[0] * dDotAb,
      ray.direction[1] - abDir[1] * dDotAb,
      ray.direction[2] - abDir[2] * dDotAb,
    ];

    const aoDotAb = ao[0] * abDir[0] + ao[1] * abDir[1] + ao[2] * abDir[2];
    const oPerp: [number, number, number] = [
      ao[0] - abDir[0] * aoDotAb,
      ao[1] - abDir[1] * aoDotAb,
      ao[2] - abDir[2] * aoDotAb,
    ];

    const a = vPerp[0] * vPerp[0] + vPerp[1] * vPerp[1] + vPerp[2] * vPerp[2];
    const b = 2.0 * (vPerp[0] * oPerp[0] + vPerp[1] * oPerp[1] + vPerp[2] * oPerp[2]);
    const c = oPerp[0] * oPerp[0] + oPerp[1] * oPerp[1] + oPerp[2] * oPerp[2] - capsule.radius * capsule.radius;

    if (a > 1e-8) {
      const discr = b * b - 4.0 * a * c;
      if (discr >= 0) {
        const sqrtDiscr = Math.sqrt(discr);
        const t0 = (-b - sqrtDiscr) / (2.0 * a);
        const t1 = (-b + sqrtDiscr) / (2.0 * a);

        for (const tCand of [t0, t1]) {
          if (tCand > 1e-5 && tCand < minT) {
            const p: [number, number, number] = [
              ray.origin[0] + ray.direction[0] * tCand,
              ray.origin[1] + ray.direction[1] * tCand,
              ray.origin[2] + ray.direction[2] * tCand,
            ];
            const proj =
              (p[0] - capsule.start[0]) * abDir[0] +
              (p[1] - capsule.start[1]) * abDir[1] +
              (p[2] - capsule.start[2]) * abDir[2];

            if (proj >= 0.0 && proj <= abLen) {
              minT = tCand;
              const axisPt: [number, number, number] = [
                capsule.start[0] + abDir[0] * proj,
                capsule.start[1] + abDir[1] * proj,
                capsule.start[2] + abDir[2] * proj,
              ];
              const diff: [number, number, number] = [p[0] - axisPt[0], p[1] - axisPt[1], p[2] - axisPt[2]];
              const diffLen = Math.hypot(diff[0], diff[1], diff[2]);
              hitNorm = diffLen > 1e-6
                ? [diff[0] / diffLen, diff[1] / diffLen, diff[2] / diffLen]
                : [0, 1, 0];
            }
          }
        }
      }
    }
  }

  if (Number.isFinite(minT) && minT > 0) {
    return {
      segment: capsule.segment,
      distance: minT,
      hitPoint: [
        ray.origin[0] + ray.direction[0] * minT,
        ray.origin[1] + ray.direction[1] * minT,
        ray.origin[2] + ray.direction[2] * minT,
      ],
      normal: hitNorm,
    };
  }

  return null;
}

export function raycastTactileHulls(ray: Ray): RaycastHit | null {
  let closest: RaycastHit | null = null;
  for (const cap of CANONICAL_HUMANOID_CAPSULES) {
    const hit = intersectCapsule(ray, cap);
    if (hit) {
      if (!closest || hit.distance < closest.distance) {
        closest = hit;
      }
    }
  }
  return closest;
}

export function createCameraRay(
  screenX: number,
  screenY: number,
  width: number,
  height: number,
  eye: [number, number, number],
  target: [number, number, number],
  up: [number, number, number],
  fov: number
): Ray {
  const ndcX = (2.0 * screenX) / width - 1.0;
  const ndcY = 1.0 - (2.0 * screenY) / height;

  const fwd: [number, number, number] = [
    target[0] - eye[0],
    target[1] - eye[1],
    target[2] - eye[2],
  ];
  const fwdLen = Math.hypot(fwd[0], fwd[1], fwd[2]);
  const w: [number, number, number] = fwdLen > 1e-6 ? [fwd[0] / fwdLen, fwd[1] / fwdLen, fwd[2] / fwdLen] : [0, 0, -1];

  let right: [number, number, number] = [
    up[1] * w[2] - up[2] * w[1],
    up[2] * w[0] - up[0] * w[2],
    up[0] * w[1] - up[1] * w[0],
  ];
  const rLen = Math.hypot(right[0], right[1], right[2]);
  const u: [number, number, number] = rLen > 1e-6 ? [right[0] / rLen, right[1] / rLen, right[2] / rLen] : [1, 0, 0];

  const v: [number, number, number] = [
    w[1] * u[2] - w[2] * u[1],
    w[2] * u[0] - w[0] * u[2],
    w[0] * u[1] - w[1] * u[0],
  ];

  const aspect = width / Math.max(height, 1);
  const halfH = Math.tan(fov / 2);
  const halfW = halfH * aspect;

  const dir: [number, number, number] = [
    w[0] + ndcX * halfW * u[0] + ndcY * halfH * v[0],
    w[1] + ndcX * halfW * u[1] + ndcY * halfH * v[1],
    w[2] + ndcX * halfW * u[2] + ndcY * halfH * v[2],
  ];
  const dLen = Math.hypot(dir[0], dir[1], dir[2]);
  const normDir: [number, number, number] = dLen > 1e-6 ? [dir[0] / dLen, dir[1] / dLen, dir[2] / dLen] : [0, 0, -1];

  return { origin: [...eye], direction: normDir };
}

export function projectTactileDrag(
  segment: AnatomicalSegment,
  dxNdc: number,
  dyNdc: number
): TactileDragResult {
  switch (segment) {
    case "Head":
      return {
        primarySlider: "head_width",
        primaryDelta: dxNdc * 0.45,
        secondarySlider: "face_lower_length",
        secondaryDelta: -dyNdc * 0.45,
      };
    case "Neck":
      return {
        primarySlider: "neck_circumference",
        primaryDelta: dxNdc * 0.50,
        secondarySlider: "neck_length",
        secondaryDelta: dyNdc * 0.50,
      };
    case "Chest":
      return {
        primarySlider: "ribcage_width",
        primaryDelta: dxNdc * 0.55,
        secondarySlider: "bust_volume_cup",
        secondaryDelta: -dyNdc * 0.65,
      };
    case "Waist":
      return {
        primarySlider: "waist_pinch_width",
        primaryDelta: dxNdc * 0.55,
        secondarySlider: "belly_visceral_protuberance",
        secondaryDelta: -dyNdc * 0.65,
      };
    case "Pelvis":
      return {
        primarySlider: "hip_trochanteric_flare",
        primaryDelta: dxNdc * 0.60,
        secondarySlider: "gluteus_volume_overall",
        secondaryDelta: -dyNdc * 0.70,
      };
    case "LeftUpperArm":
    case "RightUpperArm":
      return {
        primarySlider: "deltoid_muscle_volume",
        primaryDelta: dxNdc * 0.50,
        secondarySlider: "biceps_peak_volume",
        secondaryDelta: -dyNdc * 0.50,
      };
    case "LeftForearm":
    case "RightForearm":
      return {
        primarySlider: "forearm_brachioradialis",
        primaryDelta: dxNdc * 0.50,
        secondarySlider: "upper_arm_thickness",
        secondaryDelta: -dyNdc * 0.45,
      };
    case "LeftThigh":
    case "RightThigh":
      return {
        primarySlider: "thigh_circumference",
        primaryDelta: dxNdc * 0.55,
        secondarySlider: "inner_thigh_gap",
        secondaryDelta: -dyNdc * 0.55,
      };
    case "LeftCalf":
    case "RightCalf":
      // P0-09: fixed orphan ids (calf_gastrocnemius_volume / ankle_achilles_definition
      // never existed in the catalog) → canonical LowerLimbs sliders.
      return {
        primarySlider: "calf_circumference",
        primaryDelta: dxNdc * 0.50,
        secondarySlider: "gastrocnemius_height",
        secondaryDelta: -dyNdc * 0.50,
      };
  }
}

/**
 * P0-09: every slider id emitted by the tactile engine must exist in the
 * catalog. Throws in dev/tests when violated; the CI contract test asserts
 * zero orphans across all 13 segments.
 */
export function assertTactileIdsValid(): void {
  const segments: AnatomicalSegment[] = [
    "Head", "Neck", "Chest", "Waist", "Pelvis",
    "LeftUpperArm", "RightUpperArm", "LeftForearm", "RightForearm",
    "LeftThigh", "RightThigh", "LeftCalf", "RightCalf",
  ];
  const orphans: string[] = [];
  for (const seg of segments) {
    const r = projectTactileDrag(seg, 0.1, 0.1);
    if (!isKnownSliderId(r.primarySlider)) orphans.push(`${seg}:${r.primarySlider}`);
    if (r.secondarySlider && !isKnownSliderId(r.secondarySlider)) {
      orphans.push(`${seg}:${r.secondarySlider}`);
    }
  }
  if (orphans.length > 0) {
    throw new Error(`[tactile] orphan slider ids: ${orphans.join(", ")}`);
  }
}
