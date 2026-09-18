//! ANIGO Viewport 3D Tactile Manipulation Engine.
//! Provides raycasting tests against anatomical segment collision capsules
//! and projects screen-space cursor drag gestures to 1D parametric anatomical sliders.

use glam::{Mat4, Vec2, Vec3};
use serde::{Deserialize, Serialize};

/// Canonical anatomical segments for tactile bounding hull manipulation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AnatomicalSegment {
    Head,
    Neck,
    Chest,
    Waist,
    Pelvis,
    LeftUpperArm,
    RightUpperArm,
    LeftForearm,
    RightForearm,
    LeftThigh,
    RightThigh,
    LeftCalf,
    RightCalf,
    LeftFoot,
    RightFoot,
}

impl AnatomicalSegment {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Head => "Head",
            Self::Neck => "Neck",
            Self::Chest => "Chest",
            Self::Waist => "Waist",
            Self::Pelvis => "Pelvis",
            Self::LeftUpperArm => "LeftUpperArm",
            Self::RightUpperArm => "RightUpperArm",
            Self::LeftForearm => "LeftForearm",
            Self::RightForearm => "RightForearm",
            Self::LeftThigh => "LeftThigh",
            Self::RightThigh => "RightThigh",
            Self::LeftCalf => "LeftCalf",
            Self::RightCalf => "RightCalf",
            Self::LeftFoot => "LeftFoot",
            Self::RightFoot => "RightFoot",
        }
    }
}

/// A geometric ray in 3D world space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ray {
    pub origin: Vec3,
    pub direction: Vec3,
}

impl Ray {
    pub fn new(origin: Vec3, direction: Vec3) -> Self {
        Self {
            origin,
            direction: direction.normalize(),
        }
    }

    /// Constructs a ray from normalized device coordinates (NDC) [-1, 1] x [-1, 1]
    /// and the inverted view-projection matrix.
    pub fn from_ndc(ndc_x: f32, ndc_y: f32, inv_view_proj: Mat4, camera_eye: Vec3) -> Self {
        let near_point = inv_view_proj.project_point3(Vec3::new(ndc_x, ndc_y, 0.0));
        let far_point = inv_view_proj.project_point3(Vec3::new(ndc_x, ndc_y, 1.0));
        let dir = (far_point - near_point).normalize_or_zero();
        Self {
            origin: camera_eye,
            direction: dir,
        }
    }
}

/// Raycast intersection hit result.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RaycastHit {
    pub segment: AnatomicalSegment,
    pub distance: f32,
    pub hit_point: Vec3,
    pub normal: Vec3,
}

/// Bounding collision capsule encompassing an anatomical segment.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Capsule {
    pub segment: AnatomicalSegment,
    pub start: Vec3,
    pub end: Vec3,
    pub radius: f32,
}

impl Capsule {
    pub fn new(segment: AnatomicalSegment, start: Vec3, end: Vec3, radius: f32) -> Self {
        Self {
            segment,
            start,
            end,
            radius,
        }
    }

    /// Tests ray intersection with this capsule.
    /// Returns closest hit point with distance and normal if intersected.
    pub fn ray_intersect(&self, ray: &Ray) -> Option<RaycastHit> {
        let mut min_t = f32::INFINITY;
        let mut hit_norm = Vec3::ZERO;

        // 1. Sphere at start cap
        if let Some((t, n)) = intersect_sphere(ray, self.start, self.radius) {
            if t < min_t {
                min_t = t;
                hit_norm = n;
            }
        }

        // 2. Sphere at end cap
        if let Some((t, n)) = intersect_sphere(ray, self.end, self.radius) {
            if t < min_t {
                min_t = t;
                hit_norm = n;
            }
        }

        // 3. Finite cylinder between start and end
        let ab = self.end - self.start;
        let ab_len_sq = ab.length_squared();
        if ab_len_sq > 1e-8 {
            let ab_dir = ab.normalize();
            let ao = ray.origin - self.start;

            // Project ray and ao onto plane perpendicular to ab_dir
            let v_perp = ray.direction - ab_dir * ray.direction.dot(ab_dir);
            let o_perp = ao - ab_dir * ao.dot(ab_dir);

            let a = v_perp.length_squared();
            let b = 2.0 * v_perp.dot(o_perp);
            let c = o_perp.length_squared() - self.radius * self.radius;

            if a > 1e-8 {
                let discr = b * b - 4.0 * a * c;
                if discr >= 0.0 {
                    let sqrt_discr = discr.sqrt();
                    let t0 = (-b - sqrt_discr) / (2.0 * a);
                    let t1 = (-b + sqrt_discr) / (2.0 * a);

                    for t_cand in [t0, t1] {
                        if t_cand > 1e-5 && t_cand < min_t {
                            let p = ray.origin + ray.direction * t_cand;
                            let proj = (p - self.start).dot(ab_dir);
                            let ab_len = ab.length();
                            if proj >= 0.0 && proj <= ab_len {
                                min_t = t_cand;
                                let axis_pt = self.start + ab_dir * proj;
                                hit_norm = (p - axis_pt).normalize_or_zero();
                            }
                        }
                    }
                }
            }
        }

        if min_t.is_finite() && min_t > 0.0 {
            Some(RaycastHit {
                segment: self.segment,
                distance: min_t,
                hit_point: ray.origin + ray.direction * min_t,
                normal: hit_norm,
            })
        } else {
            None
        }
    }
}

fn intersect_sphere(ray: &Ray, center: Vec3, radius: f32) -> Option<(f32, Vec3)> {
    let oc = ray.origin - center;
    let b = oc.dot(ray.direction);
    let c = oc.length_squared() - radius * radius;
    let discr = b * b - c;

    if discr >= 0.0 {
        let sqrt_discr = discr.sqrt();
        let t0 = -b - sqrt_discr;
        let t1 = -b + sqrt_discr;

        let t = if t0 > 1e-5 {
            t0
        } else if t1 > 1e-5 {
            t1
        } else {
            return None;
        };

        let p = ray.origin + ray.direction * t;
        let normal = (p - center).normalize_or_zero();
        Some((t, normal))
    } else {
        None
    }
}

/// Collection of bounding hulls for the canonical humanoid mannequin.
#[derive(Debug, Clone)]
pub struct TactileHullSet {
    pub capsules: Vec<Capsule>,
}

impl TactileHullSet {
    pub fn new() -> Self {
        Self {
            capsules: Vec::new(),
        }
    }

    /// Constructs the canonical set of tactile capsules aligned with the base mannequin.
    pub fn canonical_humanoid() -> Self {
        let mut set = Self::new();

        // 1. Head (center y ~ 1.62, radius 0.11)
        set.capsules.push(Capsule::new(
            AnatomicalSegment::Head,
            Vec3::new(0.0, 1.55, 0.0),
            Vec3::new(0.0, 1.68, 0.0),
            0.105,
        ));

        // 2. Neck (y ~ 1.38..1.48, radius 0.055)
        set.capsules.push(Capsule::new(
            AnatomicalSegment::Neck,
            Vec3::new(0.0, 1.38, 0.0),
            Vec3::new(0.0, 1.48, 0.0),
            0.055,
        ));

        // 3. Upper Torso / Chest (y ~ 1.15..1.38, radius 0.16)
        set.capsules.push(Capsule::new(
            AnatomicalSegment::Chest,
            Vec3::new(0.0, 1.16, 0.0),
            Vec3::new(0.0, 1.36, 0.0),
            0.155,
        ));

        // 4. Lower Torso / Waist (y ~ 0.95..1.15, radius 0.12)
        set.capsules.push(Capsule::new(
            AnatomicalSegment::Waist,
            Vec3::new(0.0, 0.96, 0.0),
            Vec3::new(0.0, 1.14, 0.0),
            0.125,
        ));

        // 5. Pelvis / Hips (y ~ 0.76..0.94, radius 0.15)
        set.capsules.push(Capsule::new(
            AnatomicalSegment::Pelvis,
            Vec3::new(0.0, 0.77, 0.0),
            Vec3::new(0.0, 0.92, 0.0),
            0.150,
        ));

        // 6. Arms
        // Left Upper Arm
        set.capsules.push(Capsule::new(
            AnatomicalSegment::LeftUpperArm,
            Vec3::new(0.18, 1.36, 0.0),
            Vec3::new(0.24, 1.12, 0.0),
            0.055,
        ));
        // Right Upper Arm
        set.capsules.push(Capsule::new(
            AnatomicalSegment::RightUpperArm,
            Vec3::new(-0.18, 1.36, 0.0),
            Vec3::new(-0.24, 1.12, 0.0),
            0.055,
        ));
        // Left Forearm
        set.capsules.push(Capsule::new(
            AnatomicalSegment::LeftForearm,
            Vec3::new(0.24, 1.12, 0.0),
            Vec3::new(0.28, 0.88, 0.0),
            0.048,
        ));
        // Right Forearm
        set.capsules.push(Capsule::new(
            AnatomicalSegment::RightForearm,
            Vec3::new(-0.24, 1.12, 0.0),
            Vec3::new(-0.28, 0.88, 0.0),
            0.048,
        ));

        // 7. Legs
        // Left Thigh
        set.capsules.push(Capsule::new(
            AnatomicalSegment::LeftThigh,
            Vec3::new(0.09, 0.76, 0.0),
            Vec3::new(0.09, 0.44, 0.0),
            0.075,
        ));
        // Right Thigh
        set.capsules.push(Capsule::new(
            AnatomicalSegment::RightThigh,
            Vec3::new(-0.09, 0.76, 0.0),
            Vec3::new(-0.09, 0.44, 0.0),
            0.075,
        ));
        // Left Calf
        set.capsules.push(Capsule::new(
            AnatomicalSegment::LeftCalf,
            Vec3::new(0.09, 0.44, 0.0),
            Vec3::new(0.09, 0.12, 0.0),
            0.058,
        ));
        // Right Calf
        set.capsules.push(Capsule::new(
            AnatomicalSegment::RightCalf,
            Vec3::new(-0.09, 0.44, 0.0),
            Vec3::new(-0.09, 0.12, 0.0),
            0.058,
        ));

        set
    }

    /// Performs raycast intersection against all capsules in the hull set,
    /// returning the closest hit.
    pub fn raycast(&self, ray: &Ray) -> Option<RaycastHit> {
        let mut closest_hit: Option<RaycastHit> = None;

        for capsule in &self.capsules {
            if let Some(hit) = capsule.ray_intersect(ray) {
                if closest_hit.as_ref().map_or(true, |c| hit.distance < c.distance) {
                    closest_hit = Some(hit);
                }
            }
        }

        closest_hit
    }
}

/// Tactile projection result mapping cursor movement to anatomical sliders.
#[derive(Debug, Clone, PartialEq)]
pub struct TactileDragResult {
    pub primary_slider: &'static str,
    pub primary_delta: f32,
    pub secondary_slider: Option<&'static str>,
    pub secondary_delta: f32,
}

/// Projects a 2D screen cursor drag vector $(\Delta x, \Delta y)$ on a hit segment
/// into 1D slider delta adjustments.
pub fn project_tactile_drag(
    segment: AnatomicalSegment,
    screen_delta_ndc: Vec2, // (-1..1 range)
) -> TactileDragResult {
    let dx = screen_delta_ndc.x;
    let dy = screen_delta_ndc.y;

    match segment {
        AnatomicalSegment::Head => TactileDragResult {
            primary_slider: "head_width",
            primary_delta: dx * 0.45,
            secondary_slider: Some("face_lower_length"),
            secondary_delta: -dy * 0.45,
        },
        AnatomicalSegment::Neck => TactileDragResult {
            primary_slider: "neck_circumference",
            primary_delta: dx * 0.50,
            secondary_slider: Some("neck_length"),
            secondary_delta: dy * 0.50,
        },
        AnatomicalSegment::Chest => TactileDragResult {
            primary_slider: "ribcage_width",
            primary_delta: dx * 0.55,
            secondary_slider: Some("bust_volume_cup"),
            secondary_delta: -dy * 0.65,
        },
        AnatomicalSegment::Waist => TactileDragResult {
            primary_slider: "waist_pinch_width",
            primary_delta: dx * 0.55,
            secondary_slider: Some("belly_visceral_protuberance"),
            secondary_delta: -dy * 0.65,
        },
        AnatomicalSegment::Pelvis => TactileDragResult {
            primary_slider: "hip_trochanteric_flare",
            primary_delta: dx * 0.60,
            secondary_slider: Some("gluteus_volume_overall"),
            secondary_delta: -dy * 0.70,
        },
        AnatomicalSegment::LeftUpperArm | AnatomicalSegment::RightUpperArm => TactileDragResult {
            primary_slider: "upper_arm_thickness",
            primary_delta: dx * 0.50,
            secondary_slider: Some("upper_arm_length"),
            secondary_delta: -dy * 0.50,
        },
        AnatomicalSegment::LeftForearm | AnatomicalSegment::RightForearm => TactileDragResult {
            primary_slider: "forearm_brachioradialis",
            primary_delta: dx * 0.50,
            secondary_slider: Some("forearm_length"),
            secondary_delta: -dy * 0.50,
        },
        AnatomicalSegment::LeftThigh | AnatomicalSegment::RightThigh => TactileDragResult {
            primary_slider: "thigh_circumference",
            primary_delta: dx * 0.55,
            secondary_slider: Some("thigh_length"),
            secondary_delta: -dy * 0.55,
        },
        AnatomicalSegment::LeftCalf | AnatomicalSegment::RightCalf => TactileDragResult {
            primary_slider: "calf_circumference",
            primary_delta: dx * 0.50,
            secondary_slider: Some("leg_length_overall"),
            secondary_delta: -dy * 0.50,
        },
        AnatomicalSegment::LeftFoot | AnatomicalSegment::RightFoot => TactileDragResult {
            primary_slider: "foot_scale_and_arch",
            primary_delta: dx * 0.45,
            secondary_slider: None,
            secondary_delta: 0.0,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tactile_hull_canonical_capsules_count() {
        let hulls = TactileHullSet::canonical_humanoid();
        assert!(hulls.capsules.len() >= 13, "Must have capsules for all main humanoid segments");
    }

    #[test]
    fn test_raycast_head_capsule() {
        let hulls = TactileHullSet::canonical_humanoid();

        // Ray shot straight at head (z = 3.0 -> z = 0.0 at y = 1.62)
        let ray = Ray::new(Vec3::new(0.0, 1.62, 3.0), Vec3::new(0.0, 0.0, -1.0));
        let hit = hulls.raycast(&ray);

        assert!(hit.is_some(), "Ray directed at head must hit");
        let hit = hit.unwrap();
        assert_eq!(hit.segment, AnatomicalSegment::Head);
        assert!((hit.distance - 2.895).abs() < 0.05, "Distance should be approx 3.0 - radius (0.105)");
        assert!(hit.normal.z > 0.9, "Hit normal must point back along +Z");
    }

    #[test]
    fn test_raycast_chest_capsule() {
        let hulls = TactileHullSet::canonical_humanoid();

        // Ray shot straight at chest (y = 1.25, z = 3.0)
        let ray = Ray::new(Vec3::new(0.0, 1.25, 3.0), Vec3::new(0.0, 0.0, -1.0));
        let hit = hulls.raycast(&ray);

        assert!(hit.is_some(), "Ray directed at chest must hit");
        let hit = hit.unwrap();
        assert_eq!(hit.segment, AnatomicalSegment::Chest);
    }

    #[test]
    fn test_raycast_thigh_capsules() {
        let hulls = TactileHullSet::canonical_humanoid();

        // Left thigh at x = 0.09, y = 0.60
        let ray_left = Ray::new(Vec3::new(0.09, 0.60, 3.0), Vec3::new(0.0, 0.0, -1.0));
        let hit_left = hulls.raycast(&ray_left).expect("Must hit left thigh");
        assert_eq!(hit_left.segment, AnatomicalSegment::LeftThigh);

        // Right thigh at x = -0.09, y = 0.60
        let ray_right = Ray::new(Vec3::new(-0.09, 0.60, 3.0), Vec3::new(0.0, 0.0, -1.0));
        let hit_right = hulls.raycast(&ray_right).expect("Must hit right thigh");
        assert_eq!(hit_right.segment, AnatomicalSegment::RightThigh);
    }

    #[test]
    fn test_raycast_miss() {
        let hulls = TactileHullSet::canonical_humanoid();

        // Ray way to the side
        let ray_miss = Ray::new(Vec3::new(2.5, 1.5, 3.0), Vec3::new(0.0, 0.0, -1.0));
        let hit = hulls.raycast(&ray_miss);
        assert!(hit.is_none(), "Ray away from mannequin must miss");
    }

    #[test]
    fn test_project_tactile_drag_mappings() {
        let delta = Vec2::new(0.1, -0.2);

        let head_res = project_tactile_drag(AnatomicalSegment::Head, delta);
        assert_eq!(head_res.primary_slider, "head_width");
        assert!(head_res.primary_delta > 0.0);
        assert_eq!(head_res.secondary_slider, Some("face_lower_length"));
        assert!(head_res.secondary_delta > 0.0);

        let chest_res = project_tactile_drag(AnatomicalSegment::Chest, delta);
        assert_eq!(chest_res.primary_slider, "ribcage_width");
        assert_eq!(chest_res.secondary_slider, Some("bust_volume_cup"));

        let pelvis_res = project_tactile_drag(AnatomicalSegment::Pelvis, delta);
        assert_eq!(pelvis_res.primary_slider, "hip_trochanteric_flare");
        assert_eq!(pelvis_res.secondary_slider, Some("gluteus_volume_overall"));
    }
}
