//! ANIGO Inverse Kinematics (IK) and Skeleton Posing Engine.
//! Canonical VRM 1.0 Humanoid Rig, Kinematic Hierarchy and Bind Pose Management.

use glam::{Mat4, Quat, Vec3};
use serde::{Deserialize, Serialize};

/// Canonical Humanoid Bones conforming to VRM 1.0 and glTF standard specifications.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum HumanoidBone {
    Root,
    Hips,
    Pelvis,
    Spine,
    Chest,
    UpperChest,
    Neck,
    Head,
    LeftClavicle,
    RightClavicle,
    LeftShoulder, // UpperArm
    RightShoulder,
    LeftElbow,    // LowerArm
    RightElbow,
    LeftWrist,    // Hand
    RightWrist,
    LeftThigh,    // UpperLeg
    RightThigh,
    LeftKnee,     // LowerLeg
    RightKnee,
    LeftAnkle,    // Foot
    RightAnkle,
    LeftToes,
    RightToes,
}

impl HumanoidBone {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Root => "Root",
            Self::Hips => "Hips",
            Self::Pelvis => "Pelvis",
            Self::Spine => "Spine",
            Self::Chest => "Chest",
            Self::UpperChest => "UpperChest",
            Self::Neck => "Neck",
            Self::Head => "Head",
            Self::LeftClavicle => "LeftClavicle",
            Self::RightClavicle => "RightClavicle",
            Self::LeftShoulder => "LeftShoulder",
            Self::RightShoulder => "RightShoulder",
            Self::LeftElbow => "LeftElbow",
            Self::RightElbow => "RightElbow",
            Self::LeftWrist => "LeftWrist",
            Self::RightWrist => "RightWrist",
            Self::LeftThigh => "LeftThigh",
            Self::RightThigh => "RightThigh",
            Self::LeftKnee => "LeftKnee",
            Self::RightKnee => "RightKnee",
            Self::LeftAnkle => "LeftAnkle",
            Self::RightAnkle => "RightAnkle",
            Self::LeftToes => "LeftToes",
            Self::RightToes => "RightToes",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bone {
    pub name: String,
    pub parent_index: Option<usize>,
    pub local_position: Vec3,
    pub local_rotation: Quat,
    pub local_scale: Vec3,
    pub length: f32,
    pub humanoid_type: Option<HumanoidBone>,
}

impl Bone {
    pub fn new(
        name: impl Into<String>,
        parent_index: Option<usize>,
        local_position: Vec3,
        local_rotation: Quat,
        length: f32,
    ) -> Self {
        Self {
            name: name.into(),
            parent_index,
            local_position,
            local_rotation,
            local_scale: Vec3::ONE,
            length,
            humanoid_type: None,
        }
    }

    pub fn with_humanoid(mut self, bone_type: HumanoidBone) -> Self {
        self.humanoid_type = Some(bone_type);
        self
    }

    pub fn local_transform(&self) -> Mat4 {
        Mat4::from_scale_rotation_translation(
            self.local_scale,
            self.local_rotation,
            self.local_position,
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skeleton {
    pub name: String,
    pub bones: Vec<Bone>,
}

impl Skeleton {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            bones: Vec::new(),
        }
    }

    pub fn add_bone(&mut self, bone: Bone) -> usize {
        let index = self.bones.len();
        self.bones.push(bone);
        index
    }

    pub fn find_bone(&self, name: &str) -> Option<usize> {
        self.bones.iter().position(|b| b.name == name)
    }

    pub fn find_humanoid_bone(&self, bone_type: HumanoidBone) -> Option<usize> {
        self.bones
            .iter()
            .position(|b| b.humanoid_type == Some(bone_type))
    }

    /// Computes world transform matrices for all bones in topological order:
    /// $$W_{\text{child}} = W_{\text{parent}} \times T_{\text{local, child}}$$
    pub fn compute_world_transforms(&self) -> Vec<Mat4> {
        let mut world_transforms = Vec::with_capacity(self.bones.len());

        for bone in &self.bones {
            let local_t = bone.local_transform();
            let world_t = match bone.parent_index {
                Some(parent_idx) => {
                    let parent_w = world_transforms[parent_idx];
                    parent_w * local_t
                }
                None => local_t,
            };
            world_transforms.push(world_t);
        }

        world_transforms
    }

    /// Computes dynamic inverse bind pose matrices:
    /// $$B_{\text{inv}, i} = (W_i)^{-1}$$
    pub fn compute_inverse_bind_matrices(&self) -> Vec<Mat4> {
        let world_transforms = self.compute_world_transforms();
        world_transforms.iter().map(|w| w.inverse()).collect()
    }

    /// Computes Linear Blend Skinning (LBS) skinning palette matrices:
    /// $$S_j = W_j \times B_{\text{inv}, j}$$
    pub fn compute_skinning_palette(
        &self,
        current_world_transforms: &[Mat4],
        inverse_bind_matrices: &[Mat4],
    ) -> Vec<Mat4> {
        assert_eq!(current_world_transforms.len(), self.bones.len());
        assert_eq!(inverse_bind_matrices.len(), self.bones.len());

        current_world_transforms
            .iter()
            .zip(inverse_bind_matrices.iter())
            .map(|(w, b_inv)| *w * *b_inv)
            .collect()
    }

    /// Verifies mathematical orthogonality of bone rotation matrices (determinant == 1.0 and zero shearing).
    pub fn validate_pure_orthogonal_rotations(
        &self,
        world_transforms: &[Mat4],
    ) -> Result<(), String> {
        if world_transforms.len() != self.bones.len() {
            return Err("World transform count must match bone count".into());
        }

        for (i, (bone, w)) in self.bones.iter().zip(world_transforms.iter()).enumerate() {
            // Extract the 3x3 rotational/linear part columns
            let c0 = w.x_axis.truncate();
            let c1 = w.y_axis.truncate();
            let c2 = w.z_axis.truncate();

            // Lengths of basis vectors
            let len0 = c0.length();
            let len1 = c1.length();
            let len2 = c2.length();

            if (len0 - 1.0).abs() > 1e-4 || (len1 - 1.0).abs() > 1e-4 || (len2 - 1.0).abs() > 1e-4 {
                return Err(format!(
                    "Bone {} ({}) basis vectors are scaled: [{}, {}, {}]",
                    i, bone.name, len0, len1, len2
                ));
            }

            // Dot products between columns must be 0 (orthogonality / zero shearing)
            let dot01 = c0.dot(c1).abs();
            let dot12 = c1.dot(c2).abs();
            let dot02 = c0.dot(c2).abs();
            if dot01 > 1e-4 || dot12 > 1e-4 || dot02 > 1e-4 {
                return Err(format!(
                    "Bone {} ({}) contains shearing! dot products: [{}, {}, {}]",
                    i, bone.name, dot01, dot12, dot02
                ));
            }

            // Determinant must be 1.0 (pure rotation, no reflection or distortion)
            let det = c0.dot(c1.cross(c2));
            if (det - 1.0).abs() > 1e-4 {
                return Err(format!(
                    "Bone {} ({}) linear determinant is {} (expected 1.0)",
                    i, bone.name, det
                ));
            }
        }

        Ok(())
    }

    /// Generates canonical VRM 1.0 Humanoid Skeleton with standard reference rest pose.
    pub fn create_canonical_vrm_humanoid() -> Self {
        let mut skel = Skeleton::new("CanonicalVrmHumanoid");

        // 0: Root
        let root = Bone::new("Root", None, Vec3::ZERO, Quat::IDENTITY, 0.0)
            .with_humanoid(HumanoidBone::Root);
        let root_idx = skel.add_bone(root);

        // 1: Hips (base of torso at ~0.85m height)
        let hips = Bone::new("Hips", Some(root_idx), Vec3::new(0.0, 0.85, 0.0), Quat::IDENTITY, 0.12)
            .with_humanoid(HumanoidBone::Hips);
        let hips_idx = skel.add_bone(hips);

        // 2: Pelvis (sub-hip bone)
        let pelvis = Bone::new("Pelvis", Some(hips_idx), Vec3::new(0.0, -0.03, 0.0), Quat::IDENTITY, 0.10)
            .with_humanoid(HumanoidBone::Pelvis);
        skel.add_bone(pelvis);

        // 3: Spine
        let spine = Bone::new("Spine", Some(hips_idx), Vec3::new(0.0, 0.12, 0.0), Quat::IDENTITY, 0.14)
            .with_humanoid(HumanoidBone::Spine);
        let spine_idx = skel.add_bone(spine);

        // 4: Chest
        let chest = Bone::new("Chest", Some(spine_idx), Vec3::new(0.0, 0.14, 0.0), Quat::IDENTITY, 0.14)
            .with_humanoid(HumanoidBone::Chest);
        let chest_idx = skel.add_bone(chest);

        // 5: UpperChest
        let upper_chest = Bone::new("UpperChest", Some(chest_idx), Vec3::new(0.0, 0.14, 0.0), Quat::IDENTITY, 0.12)
            .with_humanoid(HumanoidBone::UpperChest);
        let upper_chest_idx = skel.add_bone(upper_chest);

        // 6: Neck
        let neck = Bone::new("Neck", Some(upper_chest_idx), Vec3::new(0.0, 0.12, 0.0), Quat::IDENTITY, 0.10)
            .with_humanoid(HumanoidBone::Neck);
        let neck_idx = skel.add_bone(neck);

        // 7: Head
        let head = Bone::new("Head", Some(neck_idx), Vec3::new(0.0, 0.12, 0.0), Quat::IDENTITY, 0.16)
            .with_humanoid(HumanoidBone::Head);
        skel.add_bone(head);

        // Left Arm Chain
        // 8: LeftClavicle
        let l_clav = Bone::new("LeftClavicle", Some(upper_chest_idx), Vec3::new(0.06, 0.06, 0.0), Quat::IDENTITY, 0.12)
            .with_humanoid(HumanoidBone::LeftClavicle);
        let l_clav_idx = skel.add_bone(l_clav);

        // 9: LeftShoulder (UpperArm)
        let l_shoulder = Bone::new("LeftShoulder", Some(l_clav_idx), Vec3::new(0.12, 0.0, 0.0), Quat::IDENTITY, 0.28)
            .with_humanoid(HumanoidBone::LeftShoulder);
        let l_shoulder_idx = skel.add_bone(l_shoulder);

        // 10: LeftElbow (LowerArm)
        let l_elbow = Bone::new("LeftElbow", Some(l_shoulder_idx), Vec3::new(0.28, 0.0, 0.0), Quat::IDENTITY, 0.26)
            .with_humanoid(HumanoidBone::LeftElbow);
        let l_elbow_idx = skel.add_bone(l_elbow);

        // 11: LeftWrist (Hand)
        let l_wrist = Bone::new("LeftWrist", Some(l_elbow_idx), Vec3::new(0.26, 0.0, 0.0), Quat::IDENTITY, 0.16)
            .with_humanoid(HumanoidBone::LeftWrist);
        skel.add_bone(l_wrist);

        // Right Arm Chain
        // 12: RightClavicle
        let r_clav = Bone::new("RightClavicle", Some(upper_chest_idx), Vec3::new(-0.06, 0.06, 0.0), Quat::IDENTITY, 0.12)
            .with_humanoid(HumanoidBone::RightClavicle);
        let r_clav_idx = skel.add_bone(r_clav);

        // 13: RightShoulder (UpperArm)
        let r_shoulder = Bone::new("RightShoulder", Some(r_clav_idx), Vec3::new(-0.12, 0.0, 0.0), Quat::IDENTITY, 0.28)
            .with_humanoid(HumanoidBone::RightShoulder);
        let r_shoulder_idx = skel.add_bone(r_shoulder);

        // 14: RightElbow (LowerArm)
        let r_elbow = Bone::new("RightElbow", Some(r_shoulder_idx), Vec3::new(-0.28, 0.0, 0.0), Quat::IDENTITY, 0.26)
            .with_humanoid(HumanoidBone::RightElbow);
        let r_elbow_idx = skel.add_bone(r_elbow);

        // 15: RightWrist (Hand)
        let r_wrist = Bone::new("RightWrist", Some(r_elbow_idx), Vec3::new(-0.26, 0.0, 0.0), Quat::IDENTITY, 0.16)
            .with_humanoid(HumanoidBone::RightWrist);
        skel.add_bone(r_wrist);

        // Left Leg Chain
        // 16: LeftThigh (UpperLeg)
        let l_thigh = Bone::new("LeftThigh", Some(hips_idx), Vec3::new(0.10, -0.06, 0.0), Quat::IDENTITY, 0.40)
            .with_humanoid(HumanoidBone::LeftThigh);
        let l_thigh_idx = skel.add_bone(l_thigh);

        // 17: LeftKnee (LowerLeg)
        let l_knee = Bone::new("LeftKnee", Some(l_thigh_idx), Vec3::new(0.0, -0.40, 0.0), Quat::IDENTITY, 0.38)
            .with_humanoid(HumanoidBone::LeftKnee);
        let l_knee_idx = skel.add_bone(l_knee);

        // 18: LeftAnkle (Foot)
        let l_ankle = Bone::new("LeftAnkle", Some(l_knee_idx), Vec3::new(0.0, -0.38, 0.0), Quat::IDENTITY, 0.14)
            .with_humanoid(HumanoidBone::LeftAnkle);
        let l_ankle_idx = skel.add_bone(l_ankle);

        // 19: LeftToes
        let l_toes = Bone::new("LeftToes", Some(l_ankle_idx), Vec3::new(0.0, -0.06, 0.12), Quat::IDENTITY, 0.08)
            .with_humanoid(HumanoidBone::LeftToes);
        skel.add_bone(l_toes);

        // Right Leg Chain
        // 20: RightThigh (UpperLeg)
        let r_thigh = Bone::new("RightThigh", Some(hips_idx), Vec3::new(-0.10, -0.06, 0.0), Quat::IDENTITY, 0.40)
            .with_humanoid(HumanoidBone::RightThigh);
        let r_thigh_idx = skel.add_bone(r_thigh);

        // 21: RightKnee (LowerLeg)
        let r_knee = Bone::new("RightKnee", Some(r_thigh_idx), Vec3::new(0.0, -0.40, 0.0), Quat::IDENTITY, 0.38)
            .with_humanoid(HumanoidBone::RightKnee);
        let r_knee_idx = skel.add_bone(r_knee);

        // 22: RightAnkle (Foot)
        let r_ankle = Bone::new("RightAnkle", Some(r_knee_idx), Vec3::new(0.0, -0.38, 0.0), Quat::IDENTITY, 0.14)
            .with_humanoid(HumanoidBone::RightAnkle);
        let r_ankle_idx = skel.add_bone(r_ankle);

        // 23: RightToes
        let r_toes = Bone::new("RightToes", Some(r_ankle_idx), Vec3::new(0.0, -0.06, 0.12), Quat::IDENTITY, 0.08)
            .with_humanoid(HumanoidBone::RightToes);
        skel.add_bone(r_toes);

        skel
    }

    /// Synchronizes joint translational offsets from an `anigo_core::BondSyncManager`.
    pub fn sync_from_bond(&mut self, bond_mgr: &anigo_core::bone_sync::BondSyncManager) {
        for joint in &bond_mgr.joints {
            if let Some(idx) = self.find_bone(&joint.name) {
                self.bones[idx].local_position = joint.current_local_position;
                self.bones[idx].local_rotation = joint.current_local_rotation;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_canonical_vrm_humanoid_structure() {
        let skel = Skeleton::create_canonical_vrm_humanoid();
        assert_eq!(skel.bones.len(), 24);

        // Verify root bone
        assert_eq!(skel.bones[0].humanoid_type, Some(HumanoidBone::Root));
        assert_eq!(skel.bones[0].parent_index, None);

        // Verify key humanoid bones present
        let required_bones = [
            HumanoidBone::Root,
            HumanoidBone::Hips,
            HumanoidBone::Pelvis,
            HumanoidBone::Spine,
            HumanoidBone::Chest,
            HumanoidBone::UpperChest,
            HumanoidBone::Neck,
            HumanoidBone::Head,
            HumanoidBone::LeftClavicle,
            HumanoidBone::RightClavicle,
            HumanoidBone::LeftShoulder,
            HumanoidBone::RightShoulder,
            HumanoidBone::LeftElbow,
            HumanoidBone::RightElbow,
            HumanoidBone::LeftWrist,
            HumanoidBone::RightWrist,
            HumanoidBone::LeftThigh,
            HumanoidBone::RightThigh,
            HumanoidBone::LeftKnee,
            HumanoidBone::RightKnee,
            HumanoidBone::LeftAnkle,
            HumanoidBone::RightAnkle,
            HumanoidBone::LeftToes,
            HumanoidBone::RightToes,
        ];

        for bone_type in required_bones {
            assert!(
                skel.find_humanoid_bone(bone_type).is_some(),
                "Humanoid bone {:?} must be present in canonical skeleton",
                bone_type
            );
        }
    }

    #[test]
    fn test_world_transforms_and_inverse_bind_matrices() {
        let skel = Skeleton::create_canonical_vrm_humanoid();
        let world_transforms = skel.compute_world_transforms();
        let inv_bind_matrices = skel.compute_inverse_bind_matrices();

        assert_eq!(world_transforms.len(), skel.bones.len());
        assert_eq!(inv_bind_matrices.len(), skel.bones.len());

        // B_inv * W == Identity for all bones
        for i in 0..skel.bones.len() {
            let product = inv_bind_matrices[i] * world_transforms[i];
            let diff = (product - Mat4::IDENTITY).abs();
            for col in 0..4 {
                for row in 0..4 {
                    assert!(
                        diff.col(col)[row] < 1e-4,
                        "Bone {}: B_inv * W must be Identity, element [{}, {}] diff = {}",
                        i,
                        row,
                        col,
                        diff.col(col)[row]
                    );
                }
            }
        }

        // Pure orthogonal rotations proof
        skel.validate_pure_orthogonal_rotations(&world_transforms)
            .expect("Canonical humanoid skeleton must have pure orthogonal rotations");
    }

    #[test]
    fn test_rotated_bones_preserve_orthogonality() {
        let mut skel = Skeleton::create_canonical_vrm_humanoid();

        // Apply arbitrary rotations (Euler angles / pitch / yaw / roll) to various joints
        let l_shoulder_idx = skel.find_humanoid_bone(HumanoidBone::LeftShoulder).unwrap();
        skel.bones[l_shoulder_idx].local_rotation =
            Quat::from_axis_angle(Vec3::Z, 45.0_f32.to_radians());

        let l_elbow_idx = skel.find_humanoid_bone(HumanoidBone::LeftElbow).unwrap();
        skel.bones[l_elbow_idx].local_rotation =
            Quat::from_axis_angle(Vec3::Y, 60.0_f32.to_radians());

        let head_idx = skel.find_humanoid_bone(HumanoidBone::Head).unwrap();
        skel.bones[head_idx].local_rotation =
            Quat::from_axis_angle(Vec3::X, -25.0_f32.to_radians());

        let world_transforms = skel.compute_world_transforms();
        skel.validate_pure_orthogonal_rotations(&world_transforms)
            .expect("Rotated humanoid skeleton must maintain pure orthogonal rotations without shearing");
    }

    #[test]
    fn test_skeleton_sync_from_bond_manager() {
        use anigo_core::bone_sync::{BondProportions, BondSyncManager};

        let mut bond_mgr = BondSyncManager::create_canonical_humanoid();
        let props = BondProportions {
            height_overall: 1.15,
            leg_length: 1.25,
            arm_length: 1.10,
            neck_length: 1.05,
            shoulder_width: 1.20,
            pelvis_width: 1.15,
            torso_length: 1.08,
        };
        bond_mgr.apply_proportions(&props);

        let mut skel = Skeleton::create_canonical_vrm_humanoid();
        skel.sync_from_bond(&bond_mgr);

        let world_transforms = skel.compute_world_transforms();
        let inv_bind_matrices = skel.compute_inverse_bind_matrices();

        assert_eq!(world_transforms.len(), skel.bones.len());
        assert_eq!(inv_bind_matrices.len(), skel.bones.len());

        // Validate B_inv * W == Identity
        for i in 0..skel.bones.len() {
            let prod = inv_bind_matrices[i] * world_transforms[i];
            let diff = (prod - Mat4::IDENTITY).abs();
            for col in 0..4 {
                for row in 0..4 {
                    assert!(
                        diff.col(col)[row] < 1e-4,
                        "Synced Skeleton bone {}: B_inv * W must be Identity",
                        i
                    );
                }
            }
        }

        skel.validate_pure_orthogonal_rotations(&world_transforms)
            .expect("Synced Skeleton must preserve pure orthogonal rotations without shearing");
    }

    #[test]
    fn test_skinning_palette_identity_at_rest_pose() {
        let skel = Skeleton::create_canonical_vrm_humanoid();
        let world_transforms = skel.compute_world_transforms();
        let inv_bind_matrices = skel.compute_inverse_bind_matrices();

        let palette = skel.compute_skinning_palette(&world_transforms, &inv_bind_matrices);
        assert_eq!(palette.len(), skel.bones.len());

        for (i, skin_mat) in palette.iter().enumerate() {
            let diff = (*skin_mat - Mat4::IDENTITY).abs();
            for col in 0..4 {
                for row in 0..4 {
                    assert!(
                        diff.col(col)[row] < 1e-4,
                        "Bone {}: Skinning palette matrix at rest pose must be Identity, element [{}, {}] diff = {}",
                        i,
                        row,
                        col,
                        diff.col(col)[row]
                    );
                }
            }
        }
    }
}
