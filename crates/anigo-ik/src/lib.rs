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

// ─────────────────────────────────────────────────────────────────────────────
// Fase 2 (#43): Look-At Solver com Micro-Sacadas (motor de olhos/íris)
// ─────────────────────────────────────────────────────────────────────────────
//
// O olhar é resolvido no espaço LOCAL DA CABEÇA (eixo Z local = frente do
// rosto), então a rotação dos olhos acompanha a cabeça de graça — o mesmo
// modelo matemático da projeção angular da face SDF (issue #17). Os olhos
// são nós (`LeftEye`/`RightEye`) do grafo de cena, não ossos da paleta de
// skinning (que permanece congelada em 24 ossos): o solver devolve o giro
// yaw/pitch e o integrador aplica à matriz de mundo do nó.
//
// Convenção de eixo: o olhar é o eixo +Z local do olho.
//   yaw   = rotação em volta do Y local (positivo = olhar para o +X)
//   pitch = rotação em volta do X local (positivo = olhar para cima)

/// Direção de olhar no espaço local da cabeça (radianos).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct GazeYawPitch {
    pub yaw: f32,
    pub pitch: f32,
}

impl GazeYawPitch {
    pub const ZERO: GazeYawPitch = GazeYawPitch {
        yaw: 0.0,
        pitch: 0.0,
    };

    /// Ângulo total do olhar em graus (módulo — telemetria/clamps).
    pub fn angle_degrees(self) -> f32 {
        (self.yaw * self.yaw + self.pitch * self.pitch).sqrt().to_degrees()
    }

    /// Quat que rotaciona o olho (olhar = +Z local): yaw em Y, depois pitch
    /// em X (com sinal invertido — olhar para cima é pitch positivo).
    pub fn to_quaternion(self) -> Quat {
        let yaw_q = Quat::from_axis_angle(Vec3::Y, self.yaw);
        let pitch_q = Quat::from_axis_angle(Vec3::X, -self.pitch);
        yaw_q * pitch_q
    }
}

/// Solver de rastreamento de olhar com clamps físicos e micro-sacadas
/// (ruído sutil de 2–5° que elimina o "olhar vidrado congelado" do issue).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LookAtSolver {
    /// Limite físico do olhar (radianos) — padrão 45°.
    pub max_yaw: f32,
    /// Limite físico do olhar (radianos) — padrão 35°.
    pub max_pitch: f32,
    /// Amplitude das micro-sacadas (radianos) — faixa 2–5° (padrão 2.5°).
    pub saccade_amplitude: f32,
    /// Semente determinística do ruído de sacadas.
    pub saccade_seed: f32,
}

impl Default for LookAtSolver {
    fn default() -> Self {
        Self {
            max_yaw: 45.0_f32.to_radians(),
            max_pitch: 35.0_f32.to_radians(),
            saccade_amplitude: 2.5_f32.to_radians(),
            saccade_seed: 1.23,
        }
    }
}

impl LookAtSolver {
    /// Resolve o olhar do `eye_local` (posição do olho no espaço local da
    /// cabeça — ver `EYE_OFFSET_LEFT/RIGHT`) até `target_local` (alvo no
    /// mesmo espaço), aplicando os clamps físicos dos olhos.
    pub fn solve(&self, eye_local: Vec3, target_local: Vec3) -> GazeYawPitch {
        let v = target_local - eye_local;
        let yaw = v.x.atan2(v.z);
        let horizontal = v.x.hypot(v.z);
        let pitch = v.y.atan2(horizontal);
        GazeYawPitch {
            yaw: yaw.clamp(-self.max_yaw, self.max_yaw),
            pitch: pitch.clamp(-self.max_pitch, self.max_pitch),
        }
    }

    /// Micro-sacadas: ruído temporariamente coerente (soma de 3 senoides de
    /// frequências incomensuráveis, amplitude máx = 1.0 × saccade_amplitude),
    /// determinístico em (t, seed) — suave o bastante para 60 fps, sem
    /// repetição perceptível em minutos.
    pub fn saccades(&self, time_seconds: f32) -> GazeYawPitch {
        let t = time_seconds;
        let s = self.saccade_seed;
        let ny = 0.5 * (t * 0.9 + s).sin() + 0.3 * (t * 1.7 + 2.1 * s).sin() + 0.2 * (t * 2.3 + 3.7 * s).sin();
        let np = 0.5 * (t * 1.1 + 1.3 * s).sin() + 0.3 * (t * 1.9 + 2.7 * s).sin() + 0.2 * (t * 2.9 + 4.3 * s).sin();
        GazeYawPitch {
            yaw: ny * self.saccade_amplitude,
            pitch: np * self.saccade_amplitude,
        }
    }

    /// Giro total do frame: mira + sacadas, re-clampado para o ruído nunca
    /// ultrapassar o cômodo físico dos olhos.
    pub fn frame_gaze(&self, eye_local: Vec3, target_local: Vec3, time_seconds: f32) -> GazeYawPitch {
        let aim = self.solve(eye_local, target_local);
        let sac = self.saccades(time_seconds);
        GazeYawPitch {
            yaw: (aim.yaw + sac.yaw).clamp(-self.max_yaw, self.max_yaw),
            pitch: (aim.pitch + sac.pitch).clamp(-self.max_pitch, self.max_pitch),
        }
    }
}

/// Offset do olho em relação ao osso da cabeça (espaço local; Z = frente do
/// rosto). Esquerdo = +X (lado esquerdo do personagem, que olha para +Z).
pub const EYE_OFFSET_LEFT: Vec3 = Vec3::new(0.035, -0.01, 0.09);
pub const EYE_OFFSET_RIGHT: Vec3 = Vec3::new(-0.035, -0.01, 0.09);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gaze_solver_front_and_clamps() {
        let solver = LookAtSolver::default();
        // Olho esquerdo mirando no centro do rosto (alvo em frente): convergência natural (−6.44° = -0.1124 rad).
        let front = solver.solve(EYE_OFFSET_LEFT, Vec3::new(0.0, -0.01, 0.4));
        assert!((front.yaw - (-0.11242713)).abs() < 1e-4, "convergência natural: esperado ~ -0.1124, veio {}", front.yaw);
        assert!(front.pitch.abs() < 1e-3, "olhar frontal não pode ter pitch");

        // Olho esquerdo mirando reto em frente (alvo alinhado ao olho em x=0.035) → yaw e pitch zero.
        let straight = solver.solve(EYE_OFFSET_LEFT, Vec3::new(0.035, -0.01, 0.4));
        assert!(straight.yaw.abs() < 1e-4, "olhar perfeitamente frontal ao olho deve ter yaw zero");
        assert!(straight.pitch.abs() < 1e-4, "olhar perfeitamente frontal ao olho deve ter pitch zero");

        // Alvo muito à esquerda: yaw é clamped no limite físico (45°).
        let far_left = solver.solve(EYE_OFFSET_LEFT, Vec3::new(10.0, 0.0, 0.1));
        assert!((far_left.yaw - solver.max_yaw).abs() < 1e-3, "yaw precisa clampar em +max");
        // Alvo muito acima: pitch é clamped no limite físico (35°).
        let far_up = solver.solve(EYE_OFFSET_LEFT, Vec3::new(0.035, 10.0, 0.09));
        assert!((far_up.pitch - solver.max_pitch).abs() < 1e-3, "pitch precisa clampar em +max");
    }

    #[test]
    fn test_gaze_saccades_within_amplitude_and_smooth() {
        let solver = LookAtSolver::default();
        let amplitude_deg = solver.saccade_amplitude.to_degrees();
        // Amplitude padrão dentro da faixa 2–5° do issue.
        assert!((amplitude_deg - 2.5).abs() < 1e-3);

        // |sacada| ≤ amplitude em uma varredura de 10 s a 60 fps.
        let mut previous: Option<GazeYawPitch> = None;
        for step in 0..600 {
            let t = (step as f32) / 60.0;
            let sac = solver.saccades(t);
            assert!(
                sac.yaw.abs() <= solver.saccade_amplitude + 1e-6,
                "sacada de yaw fora da amplitude em t={}",
                t
            );
            assert!(
                sac.pitch.abs() <= solver.saccade_amplitude + 1e-6,
                "sacada de pitch fora da amplitude em t={}",
                t
            );
            if let Some(prev) = previous {
                let delta = (sac.yaw - prev.yaw).abs().max((sac.pitch - prev.pitch).abs());
                assert!(
                    delta < 0.05,
                    "sacada não é suave entre frames (delta={}) em t={}",
                    delta,
                    t
                );
            }
            previous = Some(sac);
        }
    }

    #[test]
    fn test_gaze_frame_reclamps_and_quaternion_points_at_target() {
        let solver = LookAtSolver::default();
        // frame_gaze nunca ultrapassa o cômodo físico, nem com sacadas.
        for step in 0..120 {
            let t = (step as f32) / 60.0;
            let gaze = solver.frame_gaze(EYE_OFFSET_LEFT, Vec3::new(5.0, 5.0, 0.1), t);
            assert!(gaze.yaw.abs() <= solver.max_yaw + 1e-6);
            assert!(gaze.pitch.abs() <= solver.max_pitch + 1e-6);
        }

        // O quat devolve o olhar (+Z local) apontando para o alvo:
        // q * (0,0,1) ≈ normalize(alvo - olho).
        let eye = EYE_OFFSET_LEFT;
        let target = Vec3::new(0.30, 0.10, 0.80);
        let gaze = solver.solve(eye, target);
        let quat = gaze.to_quaternion();
        let forward = quat * Vec3::Z;
        let expected = (target - eye).normalize();
        assert!(
            forward.dot(expected) > 1.0 - 1e-5,
            "quat do olhar não aponta para o alvo (dot={})",
            forward.dot(expected)
        );
    }

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
