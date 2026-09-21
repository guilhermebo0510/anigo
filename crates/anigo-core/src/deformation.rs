//! ANIGO Canonical Deformation Inputs (P0 — ARQUITETURA_CANONICA_ANIGO §2.2).
//!
//! This module is the **single** place where a character's persistent inputs
//! (base gender, gender dimorphism, somatotype, macro proportions, morph
//! overrides) are turned into geometry. The viewport, the headless renderer,
//! the export path and the regression tests all consume the result through
//! [`crate::snapshot`]; no other layer is allowed to deform the canonical mesh.
//!
//! The model is explicit and versioned:
//!
//! ```text
//! base  = interpolate_gender(canonical_male, canonical_female, dimorphism)
//! base  = apply_proportions(base, proportions)          // proportion policy v1
//! delta = catalog delta per slider                      // morph catalog v1
//! p_out = p_base + Σ weight_k · delta_k                 // linear accumulation
//! n_out = normalize(n_base + Σ weight_k · delta_n_k)
//! weights:  morph overrides  (explicit, wins)
//!         + somatotype → macro sliders (somatotype policy v1)
//! ```
//!
//! `PROPORTION_POLICY_VERSION` / `SOMATOTYPE_POLICY_VERSION` are part of the
//! contract: changing a rule requires bumping the version, because saved
//! projects are expected to reproduce the same silhouette.

use serde::{Deserialize, Serialize};

use crate::mesh::{BaseGender, Mesh, Vertex};
use crate::morph_catalog::{find_slider_def, ALL_MORPH_SLIDERS};
use crate::project::{CharacterProportions, ProjectState};
use crate::somatotype::{SomatotypeCoords, SomatotypeState};

/// Version of the macro proportion policy implemented here.
pub const PROPORTION_POLICY_VERSION: u32 = 1;
/// Version of the somatotype → macro slider policy implemented here.
pub const SOMATOTYPE_POLICY_VERSION: u32 = 1;

/// Normalized height band of the head on the canonical base mesh
/// (head center ≈ 0.94 of the total height).
pub const HEAD_BAND_START: f32 = 0.86;
/// Normalized height of the hip line (legs below, torso above).
pub const HIP_BAND: f32 = 0.44;
/// Normalized height of the shoulder line.
pub const SHOULDER_BAND: f32 = 0.72;

/// Failures of the deformation input pipeline.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum DeformationError {
    /// The two canonical bases do not share topology (isomorphism broken).
    #[error("canonical bases are not isomorphic: {0}")]
    NotIsomorphic(String),
    /// A produced mesh contains non-finite values.
    #[error("deformation produced non-finite geometry in {0}")]
    NonFinite(&'static str),
}

/// Description of the inputs applied to build a character mesh.
///
/// Serialized into the snapshot so any consumer can reproduce the same
/// geometry from the same project.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeformationInputs {
    /// Version of the proportion policy.
    pub proportion_policy_version: u32,
    /// Version of the somatotype policy.
    pub somatotype_policy_version: u32,
    /// Base gender of the canonical archetype.
    pub base_gender: BaseGender,
    /// Continuous gender dimorphism (`0.0` female … `1.0` male).
    pub gender_dimorphism: f32,
    /// Normalized somatotype.
    pub somatotype: SomatotypeCoords,
    /// Macro proportions.
    pub proportions: CharacterProportions,
    /// Active sliders in catalog order: `(slider_id, weight)` with
    /// `weight = value - catalog_default`.
    pub weights: Vec<(String, f32)>,
}

impl DeformationInputs {
    /// Builds the inputs of a project (somatotype expanded to macro sliders,
    /// explicit morph overrides taking precedence over the somatotype values).
    pub fn from_project(project: &ProjectState) -> Self {
        let somatotype = SomatotypeState::new(
            project.character.somatotype,
            project.character.gender_dimorphism,
        );

        let mut weights: Vec<(String, f32)> = Vec::new();
        let mut push = |slider_id: &str, weight: f32| {
            if weight.abs() <= 1e-6 {
                return;
            }
            if let Some(existing) = weights.iter_mut().find(|(id, _)| id == slider_id) {
                existing.1 = weight;
            } else {
                weights.push((slider_id.to_string(), weight));
            }
        };

        // 1. Somatotype → macro sliders (policy v1).
        for (slider_id, value) in somatotype.resolve_macro_sliders() {
            if slider_id == "gender_dimorphism" {
                continue; // handled as base geometry, not as a morph weight
            }
            let default = find_slider_def(slider_id).map_or(0.0, |def| def.default_value);
            push(slider_id, value - default);
        }

        // 2. Explicit morph overrides win over the somatotype-derived values.
        for (slider_id, weight) in project.morph_weights() {
            push(slider_id, weight);
        }

        Self {
            proportion_policy_version: PROPORTION_POLICY_VERSION,
            somatotype_policy_version: SOMATOTYPE_POLICY_VERSION,
            base_gender: project.character.base_gender,
            gender_dimorphism: project.character.gender_dimorphism,
            somatotype: somatotype.coords,
            proportions: project.character.proportions,
            weights,
        }
    }

    /// Weight of one slider (`0.0` when the slider is at its catalog default).
    pub fn weight_of(&self, slider_id: &str) -> f32 {
        self.weights
            .iter()
            .find(|(id, _)| id == slider_id)
            .map_or(0.0, |(_, weight)| *weight)
    }

    /// Number of active (non-zero) weights.
    pub fn active_weight_count(&self) -> usize {
        self.weights
            .iter()
            .filter(|(_, weight)| weight.abs() > 1e-6)
            .count()
    }
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    if (edge1 - edge0).abs() < 1e-6 {
        return if x >= edge1 { 1.0 } else { 0.0 };
    }
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Vertical bounds of a mesh (`min_y`, `max_y`), ignoring non-finite values.
pub fn vertical_bounds(mesh: &Mesh) -> (f32, f32) {
    let mut min_y = f32::INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    for vertex in &mesh.vertices {
        let y = vertex.position[1];
        if y.is_finite() {
            min_y = min_y.min(y);
            max_y = max_y.max(y);
        }
    }
    if !min_y.is_finite() || !max_y.is_finite() {
        return (0.0, 1.0);
    }
    (min_y, max_y)
}

/// Interpolates the canonical male/female bases by gender dimorphism.
pub fn interpolate_gender(
    male: &Mesh,
    female: &Mesh,
    gender_dimorphism: f32,
) -> Result<Mesh, DeformationError> {
    if male.vertices.len() != female.vertices.len() || male.indices.len() != female.indices.len() {
        return Err(DeformationError::NotIsomorphic(format!(
            "male {} verts / {} indices vs female {} verts / {} indices",
            male.vertices.len(),
            male.indices.len(),
            female.vertices.len(),
            female.indices.len()
        )));
    }
    let state = SomatotypeState::new(
        SomatotypeCoords::default(),
        gender_dimorphism.clamp(0.0, 1.0),
    );
    state
        .interpolate_canonical_mesh(male, female)
        .map_err(DeformationError::NotIsomorphic)
}

/// Applies the macro proportion policy (v1) to a mesh.
///
/// Every rule is region-masked with a smooth falloff and is the identity at
/// default proportions, so an untouched project reproduces the canonical base
/// byte-for-byte.
pub fn apply_proportions(mesh: &Mesh, proportions: &CharacterProportions) -> Mesh {
    let mut out = mesh.clone();
    let (min_y, max_y) = vertical_bounds(mesh);
    let span = (max_y - min_y).max(1e-6);

    let height = proportions.height_overall.clamp(0.5, 2.0);
    let head_scale = proportions.head_scale.clamp(0.4, 2.5);
    let head_ratio = proportions.head_ratio.clamp(2.0, 10.0);
    let neck_length = proportions.neck_length.clamp(0.5, 2.0);
    let torso_length = proportions.torso_length.clamp(0.5, 2.0);
    let leg_length = proportions.leg_length.clamp(0.5, 2.0);
    let arm_length = proportions.arm_length.clamp(0.5, 2.0);
    let shoulder_width = proportions.shoulder_width.clamp(0.4, 2.5);

    // Head pivot: bottom of the head band.
    let head_base_y = min_y + span * HEAD_BAND_START;
    let hip_y = min_y + span * HIP_BAND;
    let shoulder_y = min_y + span * SHOULDER_BAND;

    for vertex in out.vertices.iter_mut() {
        let Vertex { position, .. } = vertex;
        let (x, y, z) = (position[0], position[1], position[2]);
        let t = (y - min_y) / span;
        let mut px = x;
        let mut py = y;
        let mut pz = z;

        // 1. Global height: uniform Y scale about the ground.
        py *= height;

        // 2. Head block: uniform scale about the head base + vertical offsets
        //    for head_ratio / neck_length.
        let head_fall = smoothstep(HEAD_BAND_START - 0.02, HEAD_BAND_START + 0.04, t);
        if head_fall > 1e-4 {
            let scaled_y = head_base_y + (py - head_base_y) * head_scale;
            let ratio_offset = (head_ratio - 6.5) / 6.5 * span * 0.10;
            let neck_offset = (neck_length - 1.0) * span * 0.05;
            let head_y = scaled_y + ratio_offset + neck_offset;
            let head_x = px * head_scale;
            let head_z = pz * head_scale;
            py = py * (1.0 - head_fall) + head_y * head_fall;
            px = px * (1.0 - head_fall) + head_x * head_fall;
            pz = pz * (1.0 - head_fall) + head_z * head_fall;
        }

        // 3. Torso band: vertical scale about the hip line.
        let torso_fall = smoothstep(HIP_BAND - 0.03, HIP_BAND + 0.03, t)
            * (1.0 - smoothstep(SHOULDER_BAND + 0.02, SHOULDER_BAND + 0.06, t));
        if torso_fall > 1e-4 {
            let torso_y = hip_y + (py - hip_y) * torso_length;
            py = py * (1.0 - torso_fall) + torso_y * torso_fall;
        }

        // 4. Legs: vertical scale about the ground.
        let leg_fall = 1.0 - smoothstep(HIP_BAND - 0.03, HIP_BAND + 0.03, t);
        if leg_fall > 1e-4 {
            let leg_y = py * leg_length;
            py = py * (1.0 - leg_fall) + leg_y * leg_fall;
        }

        // 5. Shoulders: lateral scale of the shoulder band.
        let shoulder_fall = smoothstep(SHOULDER_BAND - 0.06, SHOULDER_BAND, t)
            * (1.0 - smoothstep(SHOULDER_BAND + 0.04, SHOULDER_BAND + 0.09, t));
        if shoulder_fall > 1e-4 {
            let shoulder_x = px * shoulder_width;
            px = px * (1.0 - shoulder_fall) + shoulder_x * shoulder_fall;
            pz *= 1.0 + (shoulder_width - 1.0) * 0.35 * shoulder_fall;
        }

        // 6. Arms: vertical scale of the arm region about the shoulder line.
        if t >= 0.35 && t <= 0.90 && px.abs() > 0.10 {
            let arm_fall = smoothstep(0.10, 0.18, px.abs())
                * (1.0 - smoothstep(SHOULDER_BAND + 0.04, SHOULDER_BAND + 0.08, t));
            if arm_fall > 1e-4 {
                let arm_y = shoulder_y + (py - shoulder_y) * arm_length;
                py = py * (1.0 - arm_fall) + arm_y * arm_fall;
            }
        }

        position[0] = px;
        position[1] = py;
        position[2] = pz;
    }

    out
}

/// Recomputes smooth vertex normals from the (deformed) positions.
///
/// Same accumulation rule as the reference implementation (unit face normals
/// accumulated per incident vertex, then normalized), so the core and the
/// renderer agree on normals.
pub fn recompute_normals(mesh: &mut Mesh) {
    let count = mesh.vertices.len();
    let mut accumulator = vec![[0.0_f32; 3]; count];

    for triangle in mesh.indices.chunks_exact(3) {
        let (a, b, c) = (triangle[0] as usize, triangle[1] as usize, triangle[2] as usize);
        if a >= count || b >= count || c >= count || a == b || b == c || a == c {
            continue;
        }
        let pa = mesh.vertices[a].position;
        let pb = mesh.vertices[b].position;
        let pc = mesh.vertices[c].position;
        let ab = [pb[0] - pa[0], pb[1] - pa[1], pb[2] - pa[2]];
        let ac = [pc[0] - pa[0], pc[1] - pa[1], pc[2] - pa[2]];
        let normal = [
            ab[1] * ac[2] - ab[2] * ac[1],
            ab[2] * ac[0] - ab[0] * ac[2],
            ab[0] * ac[1] - ab[1] * ac[0],
        ];
        let length = (normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2]).sqrt();
        if length <= 1e-12 {
            continue;
        }
        let unit = [normal[0] / length, normal[1] / length, normal[2] / length];
        for index in [a, b, c] {
            accumulator[index][0] += unit[0];
            accumulator[index][1] += unit[1];
            accumulator[index][2] += unit[2];
        }
    }

    for (index, vertex) in mesh.vertices.iter_mut().enumerate() {
        let accumulated = accumulator[index];
        let length = (accumulated[0] * accumulated[0]
            + accumulated[1] * accumulated[1]
            + accumulated[2] * accumulated[2])
            .sqrt();
        if length > 1e-6 {
            vertex.normal = [
                accumulated[0] / length,
                accumulated[1] / length,
                accumulated[2] / length,
            ];
        }
    }
}

/// Builds the canonical base mesh of a gender (single source of truth).
pub fn canonical_base_mesh(gender: BaseGender) -> Mesh {
    Mesh::create_canonical_base(gender)
}

/// Builds the base geometry of a project: gender interpolation + proportions,
/// with normals recomputed from the deformed positions.
pub fn prepare_base_mesh(project: &ProjectState) -> Result<Mesh, DeformationError> {
    let male = canonical_base_mesh(BaseGender::Male);
    let female = canonical_base_mesh(BaseGender::Female);
    let mut mesh = interpolate_gender(&male, &female, project.character.gender_dimorphism)?;
    mesh = apply_proportions(&mesh, &project.character.proportions);
    recompute_normals(&mut mesh);

    for vertex in &mesh.vertices {
        if !vertex.position.iter().all(|value| value.is_finite())
            || !vertex.normal.iter().all(|value| value.is_finite())
        {
            return Err(DeformationError::NonFinite("base mesh"));
        }
    }
    Ok(mesh)
}

/// Weights per canonical slider, in catalog order, ready for the GPU channels.
pub fn catalog_weights(inputs: &DeformationInputs) -> Vec<f32> {
    ALL_MORPH_SLIDERS
        .iter()
        .map(|def| inputs.weight_of(def.id))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: f32, b: f32, tolerance: f32) -> bool {
        (a - b).abs() <= tolerance
    }

    #[test]
    fn default_proportions_are_the_identity() {
        let base = canonical_base_mesh(BaseGender::Male);
        let projected = apply_proportions(&base, &CharacterProportions::default());
        let mut max_delta: f32 = 0.0;
        for (original, result) in base.vertices.iter().zip(projected.vertices.iter()) {
            for axis in 0..3 {
                max_delta = max_delta.max((original.position[axis] - result.position[axis]).abs());
            }
        }
        assert!(
            max_delta < 1e-6,
            "identity proportions must not move geometry (max delta {max_delta})"
        );
    }

    #[test]
    fn proportions_are_deterministic_and_stay_inside_the_domain() {
        let base = canonical_base_mesh(BaseGender::Female);
        let proportions = CharacterProportions {
            head_scale: 1.35,
            head_ratio: 7.4,
            shoulder_width: 1.25,
            leg_length: 1.2,
            arm_length: 1.1,
            neck_length: 1.3,
            torso_length: 0.9,
            height_overall: 1.15,
        };
        let first = apply_proportions(&base, &proportions);
        let second = apply_proportions(&base, &proportions);
        assert_eq!(first.vertices.len(), base.vertices.len());
        assert_eq!(first.indices, base.indices);
        for (a, b) in first.vertices.iter().zip(second.vertices.iter()) {
            assert_eq!(a.position, b.position, "policy must be pure");
        }

        let (min_y, max_y) = vertical_bounds(&first);
        assert!(min_y.is_finite() && max_y.is_finite());
        assert!(
            max_y > min_y * 0.9,
            "geometry must stay a plausible body (min {min_y}, max {max_y})"
        );
        for vertex in &first.vertices {
            assert!(
                near(vertex.position[0].abs(), vertex.position[0].abs(), 10.0),
                "coordinates must stay bounded"
            );
        }
    }

    #[test]
    fn height_proportion_scales_y_monotonically() {
        let base = canonical_base_mesh(BaseGender::Male);
        let taller = apply_proportions(
            &base,
            &CharacterProportions {
                height_overall: 1.25,
                ..CharacterProportions::default()
            },
        );
        let (_, base_top) = vertical_bounds(&base);
        let (_, taller_top) = vertical_bounds(&taller);
        assert!(
            taller_top > base_top * 1.2,
            "height_overall=1.25 must scale the body (top {base_top} → {taller_top})"
        );
    }

    #[test]
    fn gender_interpolation_is_endpoint_exact_and_monotonic() {
        let male = canonical_base_mesh(BaseGender::Male);
        let female = canonical_base_mesh(BaseGender::Female);

        let as_male = interpolate_gender(&male, &female, 1.0).unwrap();
        let as_female = interpolate_gender(&male, &female, 0.0).unwrap();
        for (a, b) in as_male.vertices.iter().zip(male.vertices.iter()) {
            assert_eq!(a.position, b.position, "dimorphism 1.0 must equal the male base");
        }
        for (a, b) in as_female.vertices.iter().zip(female.vertices.iter()) {
            assert_eq!(a.position, b.position, "dimorphism 0.0 must equal the female base");
        }

        let middle = interpolate_gender(&male, &female, 0.5).unwrap();
        let male_y = male.vertices[0].position[1];
        let female_y = female.vertices[0].position[1];
        let middle_y = middle.vertices[0].position[1];
        assert!(near(middle_y, (male_y + female_y) / 2.0, 1e-5));
    }

    #[test]
    fn mismatched_topologies_are_rejected() {
        let male = canonical_base_mesh(BaseGender::Male);
        let mut female = canonical_base_mesh(BaseGender::Female);
        female.vertices.truncate(10);
        let err = interpolate_gender(&male, &female, 0.5).unwrap_err();
        assert!(matches!(err, DeformationError::NotIsomorphic(_)));
    }

    #[test]
    fn recompute_normals_yields_unit_vectors() {
        let mut mesh = canonical_base_mesh(BaseGender::Male);
        mesh.vertices[0].normal = [3.0, -4.0, 0.5]; // deliberately wrong
        recompute_normals(&mut mesh);
        for vertex in &mesh.vertices {
            let length = (vertex.normal[0] * vertex.normal[0]
                + vertex.normal[1] * vertex.normal[1]
                + vertex.normal[2] * vertex.normal[2])
                .sqrt();
            assert!(
                near(length, 1.0, 1e-3) || length < 1e-6,
                "normal length must be 1 or a zero placeholder, got {length}"
            );
        }
    }

    #[test]
    fn prepare_base_mesh_is_deterministic_and_complete() {
        let mut project = ProjectState::default();
        project.character.gender_dimorphism = 0.5;
        project.character.proportions.head_scale = 1.2;
        project.character.somatotype = SomatotypeCoords::new(0.5, 0.3, 0.2);

        let first = prepare_base_mesh(&project).unwrap();
        let second = prepare_base_mesh(&project).unwrap();
        assert_eq!(first.vertices.len(), 4070);
        assert_eq!(first.indices.len(), 20640);
        for (a, b) in first.vertices.iter().zip(second.vertices.iter()) {
            assert_eq!(a.position, b.position);
            assert_eq!(a.normal, b.normal);
        }
    }

    #[test]
    fn deformation_inputs_expand_somatotype_and_respect_explicit_overrides() {
        let mut project = ProjectState::default();
        project.character.somatotype = SomatotypeCoords::new(0.6, 0.3, 0.1);
        let inputs = DeformationInputs::from_project(&project);
        assert!(
            near(inputs.weight_of("somatotype_endomorph"), 0.6, 1e-4),
            "somatotype macro slider must receive the endomorph component"
        );
        assert!(near(inputs.weight_of("somatotype_ectomorph"), 0.1, 1e-4));
        assert_eq!(inputs.proportion_policy_version, PROPORTION_POLICY_VERSION);
        assert_eq!(inputs.somatotype_policy_version, SOMATOTYPE_POLICY_VERSION);

        // An explicit macro-slider override wins over the somatotype value.
        project.set_morph_value("somatotype_endomorph", 0.25);
        let overridden = DeformationInputs::from_project(&project);
        assert!(near(overridden.weight_of("somatotype_endomorph"), 0.25, 1e-4));

        // Defaults produce no active weights at all.
        let neutral = ProjectState::default();
        let mut neutral_inputs = DeformationInputs::from_project(&neutral);
        neutral_inputs.weights.retain(|(_, weight)| weight.abs() > 1e-6);
        assert_eq!(neutral_inputs.weights.len(), 0);
    }

    #[test]
    fn catalog_weights_follow_catalog_order() {
        let mut project = ProjectState::default();
        project.set_morph_value("head_width", 1.25);
        let inputs = DeformationInputs::from_project(&project);
        let weights = catalog_weights(&inputs);
        assert_eq!(weights.len(), ALL_MORPH_SLIDERS.len());
        let index = ALL_MORPH_SLIDERS
            .iter()
            .position(|def| def.id == "head_width")
            .unwrap();
        assert!(near(weights[index], 0.25, 1e-6));
        assert!(weights.iter().all(|weight| weight.is_finite()));
    }
}
