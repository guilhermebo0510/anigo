use glam::{Mat4, Quat, Vec3};
use serde::{Deserialize, Serialize};

/// Ossos do esqueleto humanoide canônico (VRM 1.0): 24.
///
/// É o tamanho da paleta de skinning entregue ao renderer — o WGSL declara
/// `array<mat4x4<f32>, 24>` e o contrato congela 1536 B.
pub const CANONICAL_JOINT_COUNT: usize = 24;

/// Proportion sliders for Joint Translation Offsets (BOND) system.
/// Values represent multipliers relative to canonical rest pose (1.0 = standard reference).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BondProportions {
    /// Overall vertical scale of the skeleton [0.5 .. 2.0]
    pub height_overall: f32,
    /// Multiplier for leg length (thighs, shins, ankles) [0.7 .. 1.4]
    pub leg_length: f32,
    /// Multiplier for arm length (shoulders, elbows, wrists) [0.7 .. 1.4]
    pub arm_length: f32,
    /// Multiplier for neck and head height [0.7 .. 1.4]
    pub neck_length: f32,
    /// Multiplier for biacromial shoulder width [0.7 .. 1.4]
    pub shoulder_width: f32,
    /// Multiplier for bicristal pelvis / hip width [0.7 .. 1.4]
    pub pelvis_width: f32,
    /// Multiplier for spine and torso length [0.7 .. 1.4]
    pub torso_length: f32,
}

impl Default for BondProportions {
    fn default() -> Self {
        Self {
            height_overall: 1.0,
            leg_length: 1.0,
            arm_length: 1.0,
            neck_length: 1.0,
            shoulder_width: 1.0,
            pelvis_width: 1.0,
            torso_length: 1.0,
        }
    }
}

/// A skeletal joint with reference rest transform and current modulated transform.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BondJoint {
    pub name: String,
    pub parent_index: Option<usize>,
    /// Unscaled reference rest local position
    pub rest_local_position: Vec3,
    /// Reference rest local rotation (pure unit quaternion)
    pub rest_local_rotation: Quat,
    /// Current modulated local position (updated by BOND proportions)
    pub current_local_position: Vec3,
    /// Current local rotation (preserves pure unit rotation)
    pub current_local_rotation: Quat,
    pub length: f32,
}

impl BondJoint {
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
            rest_local_position: local_position,
            rest_local_rotation: local_rotation,
            current_local_position: local_position,
            current_local_rotation: local_rotation,
            length,
        }
    }

    /// Local transformation matrix.
    /// Notice: Scales are strictly uniform 1.0 — volume is sculpted via morph targets,
    /// ensuring pure SE(3) Euclidean rigid transforms with ZERO bone shearing.
    pub fn local_transform(&self) -> Mat4 {
        Mat4::from_rotation_translation(
            self.current_local_rotation,
            self.current_local_position,
        )
    }
}

/// Joint Translation Offsets (BOND) Manager.
/// Modulates child joint translational offsets according to proportion sliders,
/// updates global Forward Kinematics world matrices recursively, and recalculates
/// dynamic inverse bind pose matrices ($B_{\text{inv}}$) ensuring pure orthogonal rotations.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct BondSyncManager {
    pub joints: Vec<BondJoint>,
}

impl BondSyncManager {
    pub fn new() -> Self {
        Self { joints: Vec::new() }
    }

    pub fn from_joints(joints: Vec<BondJoint>) -> Self {
        Self { joints }
    }

    pub fn add_joint(&mut self, joint: BondJoint) -> usize {
        let index = self.joints.len();
        self.joints.push(joint);
        index
    }

    pub fn find_joint(&self, name: &str) -> Option<usize> {
        self.joints.iter().position(|j| j.name == name)
    }

    /// Creates the canonical VRM 1.0 humanoid joint hierarchy (24 standard joints).
    pub fn create_canonical_humanoid() -> Self {
        let mut mgr = Self::new();

        // 0: Root
        let root = BondJoint::new("Root", None, Vec3::ZERO, Quat::IDENTITY, 0.0);
        let root_idx = mgr.add_joint(root);

        // 1: Hips
        let hips = BondJoint::new("Hips", Some(root_idx), Vec3::new(0.0, 0.85, 0.0), Quat::IDENTITY, 0.12);
        let hips_idx = mgr.add_joint(hips);

        // 2: Pelvis
        let pelvis = BondJoint::new("Pelvis", Some(hips_idx), Vec3::new(0.0, -0.03, 0.0), Quat::IDENTITY, 0.10);
        mgr.add_joint(pelvis);

        // 3: Spine
        let spine = BondJoint::new("Spine", Some(hips_idx), Vec3::new(0.0, 0.12, 0.0), Quat::IDENTITY, 0.14);
        let spine_idx = mgr.add_joint(spine);

        // 4: Chest
        let chest = BondJoint::new("Chest", Some(spine_idx), Vec3::new(0.0, 0.14, 0.0), Quat::IDENTITY, 0.14);
        let chest_idx = mgr.add_joint(chest);

        // 5: UpperChest
        let upper_chest = BondJoint::new("UpperChest", Some(chest_idx), Vec3::new(0.0, 0.14, 0.0), Quat::IDENTITY, 0.12);
        let upper_chest_idx = mgr.add_joint(upper_chest);

        // 6: Neck
        let neck = BondJoint::new("Neck", Some(upper_chest_idx), Vec3::new(0.0, 0.12, 0.0), Quat::IDENTITY, 0.10);
        let neck_idx = mgr.add_joint(neck);

        // 7: Head
        let head = BondJoint::new("Head", Some(neck_idx), Vec3::new(0.0, 0.12, 0.0), Quat::IDENTITY, 0.16);
        mgr.add_joint(head);

        // Left Arm Chain
        // 8: LeftClavicle
        let l_clav = BondJoint::new("LeftClavicle", Some(upper_chest_idx), Vec3::new(0.06, 0.06, 0.0), Quat::IDENTITY, 0.12);
        let l_clav_idx = mgr.add_joint(l_clav);

        // 9: LeftShoulder
        let l_shoulder = BondJoint::new("LeftShoulder", Some(l_clav_idx), Vec3::new(0.12, 0.0, 0.0), Quat::IDENTITY, 0.28);
        let l_shoulder_idx = mgr.add_joint(l_shoulder);

        // 10: LeftElbow
        let l_elbow = BondJoint::new("LeftElbow", Some(l_shoulder_idx), Vec3::new(0.28, 0.0, 0.0), Quat::IDENTITY, 0.26);
        let l_elbow_idx = mgr.add_joint(l_elbow);

        // 11: LeftWrist
        let l_wrist = BondJoint::new("LeftWrist", Some(l_elbow_idx), Vec3::new(0.26, 0.0, 0.0), Quat::IDENTITY, 0.16);
        mgr.add_joint(l_wrist);

        // Right Arm Chain
        // 12: RightClavicle
        let r_clav = BondJoint::new("RightClavicle", Some(upper_chest_idx), Vec3::new(-0.06, 0.06, 0.0), Quat::IDENTITY, 0.12);
        let r_clav_idx = mgr.add_joint(r_clav);

        // 13: RightShoulder
        let r_shoulder = BondJoint::new("RightShoulder", Some(r_clav_idx), Vec3::new(-0.12, 0.0, 0.0), Quat::IDENTITY, 0.28);
        let r_shoulder_idx = mgr.add_joint(r_shoulder);

        // 14: RightElbow
        let r_elbow = BondJoint::new("RightElbow", Some(r_shoulder_idx), Vec3::new(-0.28, 0.0, 0.0), Quat::IDENTITY, 0.26);
        let r_elbow_idx = mgr.add_joint(r_elbow);

        // 15: RightWrist
        let r_wrist = BondJoint::new("RightWrist", Some(r_elbow_idx), Vec3::new(-0.26, 0.0, 0.0), Quat::IDENTITY, 0.16);
        mgr.add_joint(r_wrist);

        // Left Leg Chain
        // 16: LeftThigh
        let l_thigh = BondJoint::new("LeftThigh", Some(hips_idx), Vec3::new(0.10, -0.06, 0.0), Quat::IDENTITY, 0.40);
        let l_thigh_idx = mgr.add_joint(l_thigh);

        // 17: LeftKnee
        let l_knee = BondJoint::new("LeftKnee", Some(l_thigh_idx), Vec3::new(0.0, -0.40, 0.0), Quat::IDENTITY, 0.38);
        let l_knee_idx = mgr.add_joint(l_knee);

        // 18: LeftAnkle
        let l_ankle = BondJoint::new("LeftAnkle", Some(l_knee_idx), Vec3::new(0.0, -0.38, 0.0), Quat::IDENTITY, 0.14);
        let l_ankle_idx = mgr.add_joint(l_ankle);

        // 19: LeftToes
        let l_toes = BondJoint::new("LeftToes", Some(l_ankle_idx), Vec3::new(0.0, -0.06, 0.12), Quat::IDENTITY, 0.08);
        mgr.add_joint(l_toes);

        // Right Leg Chain
        // 20: RightThigh
        let r_thigh = BondJoint::new("RightThigh", Some(hips_idx), Vec3::new(-0.10, -0.06, 0.0), Quat::IDENTITY, 0.40);
        let r_thigh_idx = mgr.add_joint(r_thigh);

        // 21: RightKnee
        let r_knee = BondJoint::new("RightKnee", Some(r_thigh_idx), Vec3::new(0.0, -0.40, 0.0), Quat::IDENTITY, 0.38);
        let r_knee_idx = mgr.add_joint(r_knee);

        // 22: RightAnkle
        let r_ankle = BondJoint::new("RightAnkle", Some(r_knee_idx), Vec3::new(0.0, -0.38, 0.0), Quat::IDENTITY, 0.14);
        let r_ankle_idx = mgr.add_joint(r_ankle);

        // 23: RightToes
        let r_toes = BondJoint::new("RightToes", Some(r_ankle_idx), Vec3::new(0.0, -0.06, 0.12), Quat::IDENTITY, 0.08);
        mgr.add_joint(r_toes);

        mgr
    }

    /// Applies proportion sliders by adjusting child joint translational offsets ($T_{\text{child}}$).
    ///
    /// Preserves pure Euclidean rigid rotations without scale factors or cross-axis shears:
    /// - Height: Scales vertical translation of Root/Hips and all vertical offsets.
    /// - Legs: Scales vertical offset of Thigh -> Knee -> Ankle.
    /// - Arms: Scales horizontal length of Clavicle -> Shoulder -> Elbow -> Wrist.
    /// - Neck: Scales vertical offset of UpperChest -> Neck -> Head.
    /// - Shoulders: Modulates lateral X displacement of Clavicles and Shoulders.
    /// - Pelvis: Modulates lateral X displacement of Thighs.
    pub fn apply_proportions(&mut self, props: &BondProportions) {
        let h = props.height_overall.clamp(0.4, 2.5);
        let leg = props.leg_length.clamp(0.5, 2.0);
        let arm = props.arm_length.clamp(0.5, 2.0);
        let neck = props.neck_length.clamp(0.5, 2.0);
        let shoulder_w = props.shoulder_width.clamp(0.5, 2.0);
        let pelvis_w = props.pelvis_width.clamp(0.5, 2.0);
        let torso = props.torso_length.clamp(0.5, 2.0);

        for joint in &mut self.joints {
            let mut pos = joint.rest_local_position;

            match joint.name.as_str() {
                "Hips" => {
                    // Height and leg length adjust hip height from ground
                    pos.y *= h * leg;
                }
                "Pelvis" => {
                    pos.y *= h * leg;
                }
                "Spine" | "Chest" | "UpperChest" => {
                    pos.y *= h * torso;
                }
                "Neck" | "Head" => {
                    pos.y *= h * neck;
                }
                "LeftClavicle" | "RightClavicle" => {
                    pos.x *= h * shoulder_w;
                    pos.y *= h * torso;
                }
                "LeftShoulder" | "RightShoulder" => {
                    pos.x *= h * shoulder_w;
                }
                "LeftElbow" | "LeftWrist" | "RightElbow" | "RightWrist" => {
                    pos.x *= h * arm;
                }
                "LeftThigh" | "RightThigh" => {
                    pos.x *= h * pelvis_w;
                    pos.y *= h * leg;
                }
                "LeftKnee" | "LeftAnkle" | "RightKnee" | "RightAnkle" => {
                    pos.y *= h * leg;
                }
                "LeftToes" | "RightToes" => {
                    pos.y *= h * leg;
                    pos.z *= h;
                }
                _ => {}
            }

            joint.current_local_position = pos;
        }
    }

    /// Computes world transform matrices for all joints recursively:
    /// $$W_i = W_{\text{parent}} \times T_{\text{local}, i}$$
    pub fn compute_world_transforms(&self) -> Vec<Mat4> {
        let mut world_transforms = Vec::with_capacity(self.joints.len());

        for joint in &self.joints {
            let local_t = joint.local_transform();
            let world_t = match joint.parent_index {
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

    /// Matrizes de skinning (`world × inverse bind`) na ordem dos ossos.
    ///
    /// `bind_world` são os world transforms da **pose de bind** (repouso); o
    /// estado atual do manager é a pose em vigor. Com `bind == atual` o
    /// resultado é a identidade, que é exatamente o que o núcleo precisa
    /// enquanto assa as proporções na malha base.
    pub fn skinning_matrices(&self, bind_world: &[Mat4]) -> Vec<Mat4> {
        let world = self.compute_world_transforms();
        world
            .iter()
            .enumerate()
            .map(|(index, current)| {
                let bind_inverse = bind_world
                    .get(index)
                    .copied()
                    .unwrap_or(Mat4::IDENTITY)
                    .inverse();
                *current * bind_inverse
            })
            .collect()
    }

    /// Matrizes de skinning neutras (identidade) para todos os ossos.
    pub fn identity_palette(&self) -> Vec<Mat4> {
        vec![Mat4::IDENTITY; self.joints.len()]
    }

    /// Achata as matrizes no layout do buffer do shader: 16 floats por osso, na
    /// mesma ordem de colunas que o `wgpu`/`glam` usam (`to_cols_array`).
    pub fn palette_floats(matrices: &[Mat4]) -> Vec<f32> {
        let mut floats = Vec::with_capacity(matrices.len() * 16);
        for matrix in matrices {
            floats.extend_from_slice(&matrix.to_cols_array());
        }
        floats
    }

    /// Computes dynamic inverse bind pose matrices:
    /// $$B_{\text{inv}, i} = (W_i)^{-1}$$
    pub fn compute_inverse_bind_matrices(&self) -> Vec<Mat4> {
        let world_transforms = self.compute_world_transforms();
        world_transforms.iter().map(|w| w.inverse()).collect()
    }

    /// Proves mathematical orthogonality of the rotational part ($\det(R) == 1.0$, zero shearing)
    /// for all joints across any proportion adjustments.
    pub fn validate_pure_orthogonal_rotations(
        &self,
        world_transforms: &[Mat4],
    ) -> Result<(), String> {
        if world_transforms.len() != self.joints.len() {
            return Err("World transform count must match joint count".into());
        }

        for (i, (joint, w)) in self.joints.iter().zip(world_transforms.iter()).enumerate() {
            let c0 = w.x_axis.truncate();
            let c1 = w.y_axis.truncate();
            let c2 = w.z_axis.truncate();

            let len0 = c0.length();
            let len1 = c1.length();
            let len2 = c2.length();

            if (len0 - 1.0).abs() > 1e-4 || (len1 - 1.0).abs() > 1e-4 || (len2 - 1.0).abs() > 1e-4 {
                return Err(format!(
                    "Joint {} ({}) basis vector scale violation: [{}, {}, {}]",
                    i, joint.name, len0, len1, len2
                ));
            }

            let dot01 = c0.dot(c1).abs();
            let dot12 = c1.dot(c2).abs();
            let dot02 = c0.dot(c2).abs();

            if dot01 > 1e-4 || dot12 > 1e-4 || dot02 > 1e-4 {
                return Err(format!(
                    "Joint {} ({}) shearing violation! dot products: [{}, {}, {}]",
                    i, joint.name, dot01, dot12, dot02
                ));
            }

            let det = c0.dot(c1.cross(c2));
            if (det - 1.0).abs() > 1e-4 {
                return Err(format!(
                    "Joint {} ({}) rotation determinant is {} (expected 1.0)",
                    i, joint.name, det
                ));
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_canonical_bond_manager_initialization() {
        let mgr = BondSyncManager::create_canonical_humanoid();
        assert_eq!(mgr.joints.len(), 24);

        let world_transforms = mgr.compute_world_transforms();
        assert_eq!(world_transforms.len(), 24);

        let inv_bind = mgr.compute_inverse_bind_matrices();
        assert_eq!(inv_bind.len(), 24);

        // Verify B_inv * W == Identity
        for i in 0..24 {
            let prod = inv_bind[i] * world_transforms[i];
            let diff = (prod - Mat4::IDENTITY).abs();
            for col in 0..4 {
                for row in 0..4 {
                    assert!(
                        diff.col(col)[row] < 1e-4,
                        "Joint {}: Identity mismatch at row {}, col {}: {}",
                        i,
                        row,
                        col,
                        diff.col(col)[row]
                    );
                }
            }
        }

        mgr.validate_pure_orthogonal_rotations(&world_transforms)
            .expect("Canonical rest pose must be pure orthogonal");
    }

    #[test]
    fn test_bond_proportions_extremes_preserve_pure_orthogonality() {
        let mut mgr = BondSyncManager::create_canonical_humanoid();

        // Test multiple proportion configurations from chibi to heroic
        let configs = [
            BondProportions {
                height_overall: 0.65, // Chibi
                leg_length: 0.70,
                arm_length: 0.75,
                neck_length: 0.80,
                shoulder_width: 0.80,
                pelvis_width: 0.90,
                torso_length: 0.70,
            },
            BondProportions {
                height_overall: 1.40, // Heroic 8.5c
                leg_length: 1.35,
                arm_length: 1.25,
                neck_length: 1.20,
                shoulder_width: 1.30,
                pelvis_width: 1.15,
                torso_length: 1.10,
            },
            BondProportions {
                height_overall: 1.10, // Muscular V-taper
                leg_length: 1.05,
                arm_length: 1.15,
                neck_length: 0.95,
                shoulder_width: 1.40,
                pelvis_width: 0.85,
                torso_length: 1.15,
            },
        ];

        for (cfg_idx, config) in configs.iter().enumerate() {
            mgr.apply_proportions(config);

            let world_transforms = mgr.compute_world_transforms();
            let inv_bind = mgr.compute_inverse_bind_matrices();

            mgr.validate_pure_orthogonal_rotations(&world_transforms)
                .unwrap_or_else(|err| {
                    panic!("Config {} failed pure orthogonality: {}", cfg_idx, err)
                });

            for i in 0..mgr.joints.len() {
                let prod = inv_bind[i] * world_transforms[i];
                let diff = (prod - Mat4::IDENTITY).abs();
                for col in 0..4 {
                    for row in 0..4 {
                        assert!(
                            diff.col(col)[row] < 1e-4,
                            "Config {}, Joint {}: B_inv * W != Identity",
                            cfg_idx,
                            i
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn test_bond_with_posed_joint_rotations() {
        let mut mgr = BondSyncManager::create_canonical_humanoid();

        // Apply joint rotations (e.g. T-pose to A-pose or bending knees)
        let l_shoulder_idx = mgr.find_joint("LeftShoulder").unwrap();
        mgr.joints[l_shoulder_idx].current_local_rotation =
            Quat::from_axis_angle(Vec3::Z, -40.0_f32.to_radians());

        let r_shoulder_idx = mgr.find_joint("RightShoulder").unwrap();
        mgr.joints[r_shoulder_idx].current_local_rotation =
            Quat::from_axis_angle(Vec3::Z, 40.0_f32.to_radians());

        let l_knee_idx = mgr.find_joint("LeftKnee").unwrap();
        mgr.joints[l_knee_idx].current_local_rotation =
            Quat::from_axis_angle(Vec3::X, 30.0_f32.to_radians());

        // Apply proportions
        let props = BondProportions {
            height_overall: 1.2,
            leg_length: 1.15,
            arm_length: 1.10,
            neck_length: 1.05,
            shoulder_width: 1.20,
            pelvis_width: 1.10,
            torso_length: 1.05,
        };
        mgr.apply_proportions(&props);

        let world_transforms = mgr.compute_world_transforms();
        mgr.validate_pure_orthogonal_rotations(&world_transforms)
            .expect("Posed rotated joints with BOND proportions must maintain pure orthogonality");
    }

    #[test]
    fn test_bond_feet_remain_grounded_across_all_heights() {
        let mut mgr = BondSyncManager::create_canonical_humanoid();
        let l_ankle_idx = mgr.find_joint("LeftAnkle").unwrap();
        let r_ankle_idx = mgr.find_joint("RightAnkle").unwrap();

        // Test extreme and typical height / leg proportions
        let test_cases = [
            (0.5, 1.0),
            (0.8, 1.2),
            (1.0, 1.0),
            (1.25, 0.9),
            (1.5, 1.1),
            (2.0, 1.3),
        ];

        for &(height, leg) in &test_cases {
            let props = BondProportions {
                height_overall: height,
                leg_length: leg,
                ..Default::default()
            };
            mgr.apply_proportions(&props);

            let world_transforms = mgr.compute_world_transforms();
            let l_ankle_y = world_transforms[l_ankle_idx].w_axis.y;
            let r_ankle_y = world_transforms[r_ankle_idx].w_axis.y;

            // Ankle rest height relative to ground is ~0.01 * height * leg
            assert!(
                l_ankle_y.abs() < 0.05 * height * leg + 0.02,
                "Left ankle must remain grounded (near y=0) at height={}, leg={}, got y={}",
                height,
                leg,
                l_ankle_y
            );
            assert!(
                r_ankle_y.abs() < 0.05 * height * leg + 0.02,
                "Right ankle must remain grounded (near y=0) at height={}, leg={}, got y={}",
                height,
                leg,
                r_ankle_y
            );
        }
    }
}
