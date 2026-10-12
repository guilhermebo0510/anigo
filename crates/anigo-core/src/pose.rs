//! ANIGO — modelo canônico de POSE (workspace Posing; base dos keyframes da Animação).
//!
//! ## Decisões (ver `docs/native/pesquisa/posing.md` e `docs/native/POSING_UI.md`)
//!
//! * A pose é um **offset sobre o repouso** (como o Pose Mode do Blender): por osso, a rotação local
//!   *delta* (quaternion `[x, y, z, w]`, identidade = repouso); só ossos fora da identidade são guardados
//!   (esparso, como os morphs). A translação existe apenas para `Root` e `Hips` (regra do VRM: só o
//!   quadril se translada; todos os outros ossos só giram — o que mantém cada osso uma transformação
//!   rígida SE(3), sem escala nem cisalhamento).
//! * O repouso (bind) NÃO é o T-pose do `BondSyncManager`: a malha canônica tem os braços pendurados e é
//!   mais alta que aquele esqueleto. O [`PoseRig`] guarda as posições de bind dos 24 ossos **ajustadas à
//!   malha** (`PoseRig::fit`); as rotações de bind são a identidade, então um delta de rotação é lido
//!   nos eixos do mundo quando os pais estão em repouso (X lateral, Y vertical, Z para a frente do
//!   personagem; a esquerda do personagem é +X, convenção VRM/esqueleto canônico).
//! * A paleta de skinning é `skin[i] = skin[pai] · T(b_i) · R_i · T(−b_i)` — com a pose em repouso é a
//!   identidade **exata** (bit a bit), então o frame dourado e o contrato congelado não mudam.
//! * Uma pose é editada por UM comando, [`crate::command::Command::SetPose`]; mirror, reset, aplicar
//!   ativo, colar, preset, IK e olhar são *planejadores puros* (funções deste módulo e do `anigo-ik`)
//!   que produzem a pose desejada e [`plan_pose_change`] a transforma no comando (só as diferenças).
//!
//! ## API para a Animação (keyframes)
//!
//! `PoseState` já é a unidade de um keyframe: [`PoseState::blend`] (slerp/lerp), [`PoseState::masked`],
//! [`PoseState::additive`], serialização estável (`serde`), [`PoseRig::skin_palette`] para avaliar a malha e
//! [`pose_to_asset`]/[`PoseAsset`] para guardar/trocar poses por **nome de osso**.

use std::collections::{BTreeMap, BTreeSet};

use glam::{Mat4, Quat, Vec3};
use serde::{Deserialize, Serialize};

use crate::bone_sync::CANONICAL_JOINT_COUNT;
use crate::mesh::Mesh;

/// Número de ossos da pose (= paleta de skinning congelada).
pub const BONE_COUNT: usize = CANONICAL_JOINT_COUNT;

/// Nomes canônicos dos 24 ossos (ordem do `BondSyncManager` / `skinning::joints`).
pub const BONE_NAMES: [&str; BONE_COUNT] = [
    "Root",
    "Hips",
    "Pelvis",
    "Spine",
    "Chest",
    "UpperChest",
    "Neck",
    "Head",
    "LeftClavicle",
    "LeftShoulder",
    "LeftElbow",
    "LeftWrist",
    "RightClavicle",
    "RightShoulder",
    "RightElbow",
    "RightWrist",
    "LeftThigh",
    "LeftKnee",
    "LeftAnkle",
    "LeftToes",
    "RightThigh",
    "RightKnee",
    "RightAnkle",
    "RightToes",
];

/// Pai de cada osso (`None` só na raiz).
pub const BONE_PARENTS: [Option<usize>; BONE_COUNT] = [
    None,
    Some(0),
    Some(1),
    Some(1),
    Some(3),
    Some(4),
    Some(5),
    Some(6),
    Some(5),
    Some(8),
    Some(9),
    Some(10),
    Some(5),
    Some(12),
    Some(13),
    Some(14),
    Some(1),
    Some(16),
    Some(17),
    Some(18),
    Some(1),
    Some(20),
    Some(21),
    Some(22),
];

/// Índices nomeados (mesma numeração de `skinning::joints`).
pub mod bone {
    pub const ROOT: usize = 0;
    pub const HIPS: usize = 1;
    pub const PELVIS: usize = 2;
    pub const SPINE: usize = 3;
    pub const CHEST: usize = 4;
    pub const UPPER_CHEST: usize = 5;
    pub const NECK: usize = 6;
    pub const HEAD: usize = 7;
    pub const L_CLAVICLE: usize = 8;
    pub const L_SHOULDER: usize = 9;
    pub const L_ELBOW: usize = 10;
    pub const L_WRIST: usize = 11;
    pub const R_CLAVICLE: usize = 12;
    pub const R_SHOULDER: usize = 13;
    pub const R_ELBOW: usize = 14;
    pub const R_WRIST: usize = 15;
    pub const L_THIGH: usize = 16;
    pub const L_KNEE: usize = 17;
    pub const L_ANKLE: usize = 18;
    pub const L_TOES: usize = 19;
    pub const R_THIGH: usize = 20;
    pub const R_KNEE: usize = 21;
    pub const R_ANKLE: usize = 22;
    pub const R_TOES: usize = 23;
}

/// Índice do osso pelo nome canônico.
pub fn bone_index(name: &str) -> Option<usize> {
    BONE_NAMES.iter().position(|n| *n == name)
}

/// Osso do lado oposto (X); ossos centrais devolvem a si mesmos.
pub fn mirror_bone(index: usize) -> usize {
    use bone::*;
    match index {
        L_CLAVICLE => R_CLAVICLE,
        R_CLAVICLE => L_CLAVICLE,
        L_SHOULDER => R_SHOULDER,
        R_SHOULDER => L_SHOULDER,
        L_ELBOW => R_ELBOW,
        R_ELBOW => L_ELBOW,
        L_WRIST => R_WRIST,
        R_WRIST => L_WRIST,
        L_THIGH => R_THIGH,
        R_THIGH => L_THIGH,
        L_KNEE => R_KNEE,
        R_KNEE => L_KNEE,
        L_ANKLE => R_ANKLE,
        R_ANKLE => L_ANKLE,
        L_TOES => R_TOES,
        R_TOES => L_TOES,
        other => other,
    }
}

/// `true` para ossos do lado esquerdo do personagem (+X).
pub fn is_left_bone(index: usize) -> bool {
    matches!(index, 8..=11 | 16..=19)
}

/// `true` para ossos do lado direito do personagem (−X).
pub fn is_right_bone(index: usize) -> bool {
    matches!(index, 12..=15 | 20..=23)
}

/// Filhos diretos de um osso.
pub fn bone_children(index: usize) -> Vec<usize> {
    (0..BONE_COUNT).filter(|i| BONE_PARENTS[*i] == Some(index)).collect()
}

/// Ossos do subárvore (incluindo o próprio).
pub fn bone_subtree(index: usize) -> Vec<usize> {
    let mut out = vec![index];
    let mut i = 0;
    while i < out.len() {
        let cur = out[i];
        out.extend(bone_children(cur));
        i += 1;
    }
    out
}

/// Membros com IK de dois ossos + cadeias do tronco.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Limb {
    LeftArm,
    RightArm,
    LeftLeg,
    RightLeg,
}

impl Limb {
    pub const ALL: [Limb; 4] = [Limb::LeftArm, Limb::RightArm, Limb::LeftLeg, Limb::RightLeg];

    /// `(raiz, meio, efetuador)`: ombro/cotovelo/punho ou quadril/joelho/tornozelo.
    pub fn chain(self) -> [usize; 3] {
        use bone::*;
        match self {
            Limb::LeftArm => [L_SHOULDER, L_ELBOW, L_WRIST],
            Limb::RightArm => [R_SHOULDER, R_ELBOW, R_WRIST],
            Limb::LeftLeg => [L_THIGH, L_KNEE, L_ANKLE],
            Limb::RightLeg => [R_THIGH, R_KNEE, R_ANKLE],
        }
    }

    pub fn mirror(self) -> Limb {
        match self {
            Limb::LeftArm => Limb::RightArm,
            Limb::RightArm => Limb::LeftArm,
            Limb::LeftLeg => Limb::RightLeg,
            Limb::RightLeg => Limb::LeftLeg,
        }
    }

    pub fn is_arm(self) -> bool {
        matches!(self, Limb::LeftArm | Limb::RightArm)
    }

    pub fn is_left(self) -> bool {
        matches!(self, Limb::LeftArm | Limb::LeftLeg)
    }

    /// Membro cujo osso do efetuador/meio/raiz contém `bone`.
    pub fn of_bone(index: usize) -> Option<Limb> {
        Limb::ALL.into_iter().find(|l| l.chain().contains(&index))
    }

    /// Chave estável (`left_arm`…) para persistência de preferências.
    pub fn key(self) -> &'static str {
        match self {
            Limb::LeftArm => "left_arm",
            Limb::RightArm => "right_arm",
            Limb::LeftLeg => "left_leg",
            Limb::RightLeg => "right_leg",
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Estado da pose
// ---------------------------------------------------------------------------------------------

/// Curvatura dos dedos de uma mão (0 = aberto, 1 = fechado): polegar, indicador, médio, anelar, mínimo.
pub type FingerCurls = [f32; 5];

/// Pose das duas mãos (o esqueleto canônico não tem ossos de dedo: ver `finger_curl_mesh`).
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct HandsPose {
    #[serde(default)]
    pub left: FingerCurls,
    #[serde(default)]
    pub right: FingerCurls,
}

impl HandsPose {
    pub fn is_rest(&self) -> bool {
        self.left.iter().chain(self.right.iter()).all(|v| v.abs() <= 1e-6)
    }

    pub fn mirrored(&self) -> HandsPose {
        HandsPose { left: self.right, right: self.left }
    }

    pub fn blend(a: &HandsPose, b: &HandsPose, t: f32) -> HandsPose {
        let mix = |x: FingerCurls, y: FingerCurls| {
            let mut out = [0.0; 5];
            for i in 0..5 {
                out[i] = x[i] + (y[i] - x[i]) * t;
            }
            out
        };
        HandsPose { left: mix(a.left, b.left), right: mix(a.right, b.right) }
    }

    fn validate(&self) -> Result<(), String> {
        for v in self.left.iter().chain(self.right.iter()) {
            if !v.is_finite() || !(-1e-4..=1.0 + 1e-4).contains(v) {
                return Err(format!("curvatura de dedo fora de [0, 1]: {v}"));
            }
        }
        Ok(())
    }
}

/// Limite de deslocamento do quadril/raiz (metros): evita valores absurdos vindos de arquivos.
pub const MAX_TRANSLATION: f32 = 50.0;

fn is_zero3(v: &[f32; 3]) -> bool {
    v.iter().all(|c| c.abs() <= 1e-7)
}

fn hands_rest(h: &HandsPose) -> bool {
    h.is_rest()
}

/// Pose do personagem: deltas de rotação local por osso + translação do root/quadril + dedos.
///
/// Campo aditivo de `CharacterState` (`serde(default)`, omitido em repouso).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct PoseState {
    /// Delta de rotação local por nome canônico de osso (`[x, y, z, w]`); ausente = identidade.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub rotations: BTreeMap<String, [f32; 4]>,
    /// Translação do osso `Root` (move o personagem inteiro; pés incluídos).
    #[serde(default, skip_serializing_if = "is_zero3")]
    pub root_translation: [f32; 3],
    /// Translação do osso `Hips` relativa ao repouso (corpo se move, pés ficam se houver IK).
    #[serde(default, skip_serializing_if = "is_zero3")]
    pub hips_translation: [f32; 3],
    /// Curvatura dos dedos.
    #[serde(default, skip_serializing_if = "hands_rest")]
    pub hands: HandsPose,
}

/// Um osso de um [`crate::command::Command::SetPose`]: rotação delta final do osso.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PoseBoneEntry {
    pub bone: String,
    pub rotation: [f32; 4],
}

/// Rótulo da operação (só descrição/telemetria do histórico; o efeito é sempre o das entradas).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PoseOp {
    #[default]
    Edit,
    Mirror,
    Reset,
    Apply,
    Paste,
    Preset,
    Ik,
    Gaze,
    Hand,
    Ground,
}

impl PoseOp {
    pub fn label(self) -> &'static str {
        match self {
            PoseOp::Edit => "edit",
            PoseOp::Mirror => "mirror",
            PoseOp::Reset => "reset",
            PoseOp::Apply => "apply",
            PoseOp::Paste => "paste",
            PoseOp::Preset => "preset",
            PoseOp::Ik => "ik",
            PoseOp::Gaze => "gaze",
            PoseOp::Hand => "hand",
            PoseOp::Ground => "ground",
        }
    }
}

const ROT_EPS: f32 = 1e-6;

fn q_from(a: [f32; 4]) -> Quat {
    let q = Quat::from_xyzw(a[0], a[1], a[2], a[3]);
    if q.is_finite() && q.length_squared() > 1e-12 {
        q.normalize()
    } else {
        Quat::IDENTITY
    }
}

fn q_is_identity(q: Quat) -> bool {
    // identidade a menos do sinal (q e −q são a mesma rotação)
    (q.w.abs() - 1.0).abs() <= ROT_EPS && q.x.abs() <= ROT_EPS && q.y.abs() <= ROT_EPS && q.z.abs() <= ROT_EPS
}

impl PoseState {
    /// `true` quando nada foge do repouso.
    pub fn is_rest(&self) -> bool {
        self.rotations.is_empty() && is_zero3(&self.root_translation) && is_zero3(&self.hips_translation) && self.hands.is_rest()
    }

    /// Rotação delta do osso (identidade se ausente).
    pub fn rotation(&self, bone: usize) -> Quat {
        BONE_NAMES.get(bone).and_then(|n| self.rotations.get(*n)).map(|a| q_from(*a)).unwrap_or(Quat::IDENTITY)
    }

    /// Define a rotação delta do osso (identidade remove a entrada). Normaliza.
    pub fn set_rotation(&mut self, bone: usize, q: Quat) {
        let Some(name) = BONE_NAMES.get(bone) else { return };
        let q = if q.is_finite() && q.length_squared() > 1e-12 { q.normalize() } else { Quat::IDENTITY };
        if q_is_identity(q) {
            self.rotations.remove(*name);
        } else {
            // canonicaliza o sinal (w >= 0) para a igualdade/serialização serem estáveis
            let q = if q.w < 0.0 { -q } else { q };
            self.rotations.insert((*name).to_string(), q.to_array());
        }
    }

    /// Translação efetiva de um osso (só `Root` e `Hips` têm).
    pub fn translation(&self, bone: usize) -> Vec3 {
        match bone {
            bone::ROOT => Vec3::from_array(self.root_translation),
            bone::HIPS => Vec3::from_array(self.hips_translation),
            _ => Vec3::ZERO,
        }
    }

    /// Ossos com rotação diferente da identidade.
    pub fn posed_bones(&self) -> Vec<usize> {
        self.rotations.keys().filter_map(|n| bone_index(n)).collect()
    }

    /// Valida nomes, finitude e faixas. Mesma regra do comando e do `ProjectState::validate`.
    pub fn validate(&self) -> Result<(), String> {
        for (name, q) in &self.rotations {
            if bone_index(name).is_none() {
                return Err(format!("osso desconhecido '{name}'"));
            }
            if q.iter().any(|c| !c.is_finite()) {
                return Err(format!("rotação não finita em '{name}'"));
            }
            let len2: f32 = q.iter().map(|c| c * c).sum();
            if !(0.25..=4.0).contains(&len2) {
                return Err(format!("quaternion de '{name}' não é unitário (|q|² = {len2})"));
            }
        }
        for (field, t) in [("root_translation", self.root_translation), ("hips_translation", self.hips_translation)] {
            if t.iter().any(|c| !c.is_finite() || c.abs() > MAX_TRANSLATION) {
                return Err(format!("{field} fora de ±{MAX_TRANSLATION} m"));
            }
        }
        self.hands.validate()
    }

    /// Repara uma pose lida de arquivo: descarta ossos desconhecidos/não finitos, renormaliza, limita translações.
    pub fn sanitize(&mut self) {
        let mut fixed = BTreeMap::new();
        for (name, q) in std::mem::take(&mut self.rotations) {
            if bone_index(&name).is_none() || q.iter().any(|c| !c.is_finite()) {
                continue;
            }
            let quat = q_from(q);
            if !q_is_identity(quat) {
                let quat = if quat.w < 0.0 { -quat } else { quat };
                fixed.insert(name, quat.to_array());
            }
        }
        self.rotations = fixed;
        for t in [&mut self.root_translation, &mut self.hips_translation] {
            for c in t.iter_mut() {
                *c = if c.is_finite() { c.clamp(-MAX_TRANSLATION, MAX_TRANSLATION) } else { 0.0 };
            }
        }
        for v in self.hands.left.iter_mut().chain(self.hands.right.iter_mut()) {
            *v = if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 };
        }
    }

    /// Espelha a pose no plano X (esquerda ↔ direita; ossos centrais refletidos): `q' = (x, −y, −z, w)`,
    /// translações com `x → −x`, mãos trocadas.
    pub fn mirrored(&self) -> PoseState {
        let mut out = PoseState::default();
        for bone in 0..BONE_COUNT {
            let q = self.rotation(bone);
            let m = Quat::from_xyzw(q.x, -q.y, -q.z, q.w);
            out.set_rotation(mirror_bone(bone), m);
        }
        let flip = |t: [f32; 3]| [-t[0], t[1], t[2]];
        out.root_translation = flip(self.root_translation);
        out.hips_translation = flip(self.hips_translation);
        out.hands = self.hands.mirrored();
        out
    }

    /// Mistura `a → b` (slerp nos ossos, lerp nas translações e dedos). `t` pode extrapolar fora de `[0, 1]`
    /// (rotações extrapoladas continuam unitárias: o ângulo é ampliado).
    pub fn blend(a: &PoseState, b: &PoseState, t: f32) -> PoseState {
        let mut out = PoseState::default();
        for bone in 0..BONE_COUNT {
            let (qa, qb) = (a.rotation(bone), b.rotation(bone));
            if q_is_identity(qa) && q_is_identity(qb) {
                continue;
            }
            // delta = qb·qa⁻¹ elevado a t, aplicado sobre qa: permite extrapolar de forma estável
            let qb = if qa.dot(qb) < 0.0 { -qb } else { qb };
            let delta = qb * qa.inverse();
            let (axis, angle) = delta.to_axis_angle();
            let scaled = if axis.is_finite() && angle.abs() > 1e-9 { Quat::from_axis_angle(axis, angle * t) } else { Quat::IDENTITY };
            out.set_rotation(bone, scaled * qa);
        }
        let lerp3 = |x: [f32; 3], y: [f32; 3]| [x[0] + (y[0] - x[0]) * t, x[1] + (y[1] - x[1]) * t, x[2] + (y[2] - x[2]) * t];
        out.root_translation = lerp3(a.root_translation, b.root_translation);
        out.hips_translation = lerp3(a.hips_translation, b.hips_translation);
        let mut hands = HandsPose::blend(&a.hands, &b.hands, t);
        for v in hands.left.iter_mut().chain(hands.right.iter_mut()) {
            *v = v.clamp(0.0, 1.0);
        }
        out.hands = hands;
        out
    }

    /// Só os ossos em `mask` (translações e mãos seguem se o osso respectivo estiver na máscara).
    pub fn masked(&self, mask: &BTreeSet<usize>) -> PoseState {
        let mut out = PoseState::default();
        for &bone in mask {
            out.set_rotation(bone, self.rotation(bone));
        }
        if mask.contains(&bone::ROOT) {
            out.root_translation = self.root_translation;
        }
        if mask.contains(&bone::HIPS) {
            out.hips_translation = self.hips_translation;
        }
        if mask.contains(&bone::L_WRIST) {
            out.hands.left = self.hands.left;
        }
        if mask.contains(&bone::R_WRIST) {
            out.hands.right = self.hands.right;
        }
        out
    }

    /// `base` com `other` por cima só nos ossos que `other` pisa (`other_mask`).
    pub fn overlay(&self, other: &PoseState, mask: &BTreeSet<usize>) -> PoseState {
        let mut out = self.clone();
        let m = other.masked(mask);
        for &bone in mask {
            out.set_rotation(bone, m.rotation(bone));
        }
        if mask.contains(&bone::ROOT) {
            out.root_translation = m.root_translation;
        }
        if mask.contains(&bone::HIPS) {
            out.hips_translation = m.hips_translation;
        }
        if mask.contains(&bone::L_WRIST) {
            out.hands.left = m.hands.left;
        }
        if mask.contains(&bone::R_WRIST) {
            out.hands.right = m.hands.right;
        }
        out
    }

    /// Soma aditiva: `self · delta` por osso (delta aplicado depois), translações somadas.
    pub fn additive(&self, delta: &PoseState) -> PoseState {
        let mut out = PoseState::default();
        for bone in 0..BONE_COUNT {
            out.set_rotation(bone, self.rotation(bone) * delta.rotation(bone));
        }
        for i in 0..3 {
            out.root_translation[i] = self.root_translation[i] + delta.root_translation[i];
            out.hips_translation[i] = self.hips_translation[i] + delta.hips_translation[i];
        }
        out.hands = self.hands;
        out
    }

    /// Conjunto de ossos que diferem de `other` (rotação ou translação).
    pub fn differing_bones(&self, other: &PoseState) -> BTreeSet<usize> {
        let mut out = BTreeSet::new();
        for bone in 0..BONE_COUNT {
            if self.rotation(bone).dot(other.rotation(bone)).abs() < 1.0 - 1e-9 {
                out.insert(bone);
            }
        }
        if self.root_translation != other.root_translation {
            out.insert(bone::ROOT);
        }
        if self.hips_translation != other.hips_translation {
            out.insert(bone::HIPS);
        }
        out
    }

    /// Aplica o conteúdo de um `SetPose` (rotações por osso + opcionais).
    pub(crate) fn apply_patch(&mut self, bones: &[PoseBoneEntry], root: Option<[f32; 3]>, hips: Option<[f32; 3]>, hands: Option<HandsPose>) {
        for e in bones {
            if let Some(i) = bone_index(&e.bone) {
                self.set_rotation(i, q_from(e.rotation));
            }
        }
        if let Some(t) = root {
            self.root_translation = t;
        }
        if let Some(t) = hips {
            self.hips_translation = t;
        }
        if let Some(h) = hands {
            self.hands = h;
        }
    }
}

/// Planeja o `SetPose` que leva `from` até `to` (só o que difere). `None` quando nada muda.
pub fn plan_pose_change(from: &PoseState, to: &PoseState, op: PoseOp) -> Option<crate::command::Command> {
    let mut bones = Vec::new();
    for bone in 0..BONE_COUNT {
        let (a, b) = (from.rotation(bone), to.rotation(bone));
        if a.dot(b).abs() < 1.0 - 1e-9 {
            bones.push(PoseBoneEntry { bone: BONE_NAMES[bone].to_string(), rotation: b.to_array() });
        }
    }
    let root = (from.root_translation != to.root_translation).then_some(to.root_translation);
    let hips = (from.hips_translation != to.hips_translation).then_some(to.hips_translation);
    let hands = (from.hands != to.hands).then_some(to.hands);
    if bones.is_empty() && root.is_none() && hips.is_none() && hands.is_none() {
        return None;
    }
    Some(crate::command::Command::SetPose { bones, root_translation: root, hips_translation: hips, hands, op })
}

// ---------------------------------------------------------------------------------------------
// Limites articulares (swing-twist)
// ---------------------------------------------------------------------------------------------

/// Perfil de limites: o que o artista vê como "Humano / Anime / Livre".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LimitProfile {
    /// Amplitudes normativas (AAOS/Physiopedia, repartidas entre os ossos).
    #[default]
    Human,
    /// ×1,25 em swing/twist (anime exagera dobras e arqueamentos).
    Anime,
    /// Sem limites.
    Free,
}

impl LimitProfile {
    pub const ALL: [LimitProfile; 3] = [LimitProfile::Human, LimitProfile::Anime, LimitProfile::Free];

    pub fn scale(self) -> Option<f32> {
        match self {
            LimitProfile::Human => Some(1.0),
            LimitProfile::Anime => Some(1.25),
            LimitProfile::Free => None,
        }
    }
}

/// Limite swing-twist de um osso (graus). O eixo de torção é o **Y local**; o swing é o vetor de rotação
/// `(θx, 0, θz)` que leva o eixo Y à posição final (cone elíptico com extensões assimétricas).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BoneLimit {
    /// Faixa de torção em torno de Y.
    pub twist: [f32; 2],
    /// Faixa do swing em torno de X (sinal de θx).
    pub sx: [f32; 2],
    /// Faixa do swing em torno de Z (sinal de θz).
    pub sz: [f32; 2],
}

impl BoneLimit {
    const fn new(sx: [f32; 2], sz: [f32; 2], twist: [f32; 2]) -> Self {
        BoneLimit { twist, sx, sz }
    }

    fn mirrored(self) -> BoneLimit {
        BoneLimit { twist: [-self.twist[1], -self.twist[0]], sx: self.sx, sz: [-self.sz[1], -self.sz[0]] }
    }

    fn scaled(self, k: f32) -> BoneLimit {
        let s = |r: [f32; 2]| [r[0] * k, r[1] * k];
        BoneLimit { twist: s(self.twist), sx: s(self.sx), sz: s(self.sz) }
    }
}

/// Limite do osso no perfil. `None` = sem limite (raiz ou perfil livre).
pub fn bone_limit(bone: usize, profile: LimitProfile) -> Option<BoneLimit> {
    use bone::*;
    let k = profile.scale()?;
    // Valores do lado ESQUERDO (+X); o direito é o espelho. Convenção dos eixos (rest com braços/pernas pendurados):
    // +θx em osso que aponta para −Y leva o membro para trás; +θz leva-o para +X (esquerda).
    let left = |i: usize| -> Option<BoneLimit> {
        Some(match i {
            HIPS => BoneLimit::new([-110.0, 110.0], [-70.0, 70.0], [-110.0, 110.0]),
            PELVIS => BoneLimit::new([-15.0, 15.0], [-10.0, 10.0], [-10.0, 10.0]),
            SPINE => BoneLimit::new([-10.0, 20.0], [-8.0, 8.0], [-10.0, 10.0]),
            CHEST => BoneLimit::new([-8.0, 15.0], [-8.0, 8.0], [-10.0, 10.0]),
            UPPER_CHEST => BoneLimit::new([-7.0, 15.0], [-9.0, 9.0], [-10.0, 10.0]),
            NECK => BoneLimit::new([-35.0, 30.0], [-22.0, 22.0], [-40.0, 40.0]),
            HEAD => BoneLimit::new([-40.0, 30.0], [-23.0, 23.0], [-40.0, 40.0]),
            L_CLAVICLE => BoneLimit::new([-5.0, 5.0], [-12.0, 20.0], [-15.0, 10.0]),
            L_SHOULDER => BoneLimit::new([-180.0, 50.0], [-50.0, 180.0], [-90.0, 90.0]),
            L_ELBOW => BoneLimit::new([-150.0, 3.0], [-12.0, 12.0], [-85.0, 85.0]), // sz: folga p/ a inclinação do osso de repouso do braço
            L_WRIST => BoneLimit::new([-60.0, 60.0], [-20.0, 20.0], [-20.0, 20.0]),
            L_THIGH => BoneLimit::new([-120.0, 30.0], [-20.0, 45.0], [-45.0, 45.0]),
            L_KNEE => BoneLimit::new([-3.0, 150.0], [-3.0, 3.0], [-8.0, 8.0]),
            L_ANKLE => BoneLimit::new([-25.0, 45.0], [-20.0, 20.0], [-15.0, 15.0]),
            L_TOES => BoneLimit::new([-60.0, 35.0], [-5.0, 5.0], [-5.0, 5.0]),
            _ => return None,
        })
    };
    let lim = if is_right_bone(bone) { left(mirror_bone(bone))?.mirrored() } else { left(bone)? };
    Some(lim.scaled(k))
}

/// Decomposição swing-twist em torno de Y: `q = swing · twist`. Devolve `(θx, θz, ângulo de torção)` em radianos.
pub fn swing_twist(q: Quat) -> (f32, f32, f32) {
    let q = if q.w < 0.0 { -q } else { q }.normalize();
    let tw = Quat::from_xyzw(0.0, q.y, 0.0, q.w);
    let twist_angle;
    let twist = if tw.length_squared() > 1e-12 {
        let t = tw.normalize();
        twist_angle = 2.0 * t.y.atan2(t.w);
        t
    } else {
        twist_angle = 0.0;
        Quat::IDENTITY
    };
    let swing = q * twist.inverse();
    let swing = if swing.w < 0.0 { -swing } else { swing };
    let v = Vec3::new(swing.x, 0.0, swing.z);
    let s = v.length();
    if s < 1e-9 {
        return (0.0, 0.0, twist_angle);
    }
    let angle = 2.0 * s.atan2(swing.w);
    let axis = v / s;
    (axis.x * angle, axis.z * angle, twist_angle)
}

/// Recompõe a rotação a partir de `(θx, θz, torção)`.
pub fn from_swing_twist(tx: f32, tz: f32, twist: f32) -> Quat {
    let v = Vec3::new(tx, 0.0, tz);
    let a = v.length();
    let swing = if a > 1e-9 { Quat::from_axis_angle(v / a, a) } else { Quat::IDENTITY };
    (swing * Quat::from_rotation_y(twist)).normalize()
}

/// Limita `q` ao [`BoneLimit`]. Idempotente e contínuo; rotações dentro do limite voltam inalteradas.
pub fn clamp_to_limit(q: Quat, limit: &BoneLimit) -> Quat {
    let (tx, tz, tw) = swing_twist(q);
    let (txd, tzd, twd) = (tx.to_degrees(), tz.to_degrees(), tw.to_degrees());
    // twist
    let twd_c = twd.clamp(limit.twist[0], limit.twist[1]);
    // swing: cone elíptico assimétrico
    let ex = if txd >= 0.0 { limit.sx[1] } else { -limit.sx[0] }.max(1e-3);
    let ez = if tzd >= 0.0 { limit.sz[1] } else { -limit.sz[0] }.max(1e-3);
    let (u, v) = (txd / ex, tzd / ez);
    let r2 = u * u + v * v;
    let (txd_c, tzd_c) = if r2 > 1.0 {
        let k = 1.0 / r2.sqrt();
        (txd * k, tzd * k)
    } else {
        (txd, tzd)
    };
    if (twd_c - twd).abs() < 1e-5 && (txd_c - txd).abs() < 1e-5 && (tzd_c - tzd).abs() < 1e-5 {
        return q.normalize();
    }
    from_swing_twist(txd_c.to_radians(), tzd_c.to_radians(), twd_c.to_radians())
}

/// `true` quando `q` já respeita o limite (com tolerância).
pub fn within_limit(q: Quat, limit: &BoneLimit) -> bool {
    let c = clamp_to_limit(q, limit);
    c.dot(q.normalize()).abs() > 1.0 - 1e-6
}

/// Aplica os limites do perfil a todos os ossos da pose.
pub fn clamp_pose(pose: &PoseState, profile: LimitProfile) -> PoseState {
    let mut out = pose.clone();
    for bone in 0..BONE_COUNT {
        if let Some(lim) = bone_limit(bone, profile) {
            let q = pose.rotation(bone);
            out.set_rotation(bone, clamp_to_limit(q, &lim));
        }
    }
    out
}

/// Euler XYZ em graus (só para o painel numérico; a pose é guardada como quaternion).
pub fn quat_to_euler_deg(q: Quat) -> [f32; 3] {
    let (x, y, z) = q.to_euler(glam::EulerRot::XYZ);
    [x.to_degrees(), y.to_degrees(), z.to_degrees()]
}

/// Inverso de [`quat_to_euler_deg`].
pub fn euler_deg_to_quat(e: [f32; 3]) -> Quat {
    Quat::from_euler(glam::EulerRot::XYZ, e[0].to_radians(), e[1].to_radians(), e[2].to_radians())
}

// ---------------------------------------------------------------------------------------------
// Rig ajustado à malha
// ---------------------------------------------------------------------------------------------

/// Posições de bind canônicas (espaço da malha canônica, metros) para um gênero:
/// índice = osso. Derivadas das constantes do gerador `Mesh::create_canonical_base`
/// (cada junta no encontro das partes do corpo; braços pendurados, pernas retas).
fn canonical_bind_gender(male: bool) -> [Vec3; BONE_COUNT] {
    // (masculino, feminino) por quantidade
    let g = |m: f32, f: f32| if male { m } else { f };
    let hips = g(0.90, 0.82);
    let spine = g(1.10, 1.02);
    let chest = g(1.25, 1.15);
    let upper = g(1.37, 1.26);
    let neck = g(1.46, 1.36);
    let head = g(1.57, 1.46);
    let clav_x = 0.04;
    let clav_y = g(1.40, 1.30);
    let sh_x = g(0.23, 0.18);
    let sh_y = g(1.42, 1.32);
    let el_y = g(1.15, 1.08);
    let el_x = g(0.24, 0.19);
    let wr_y = g(0.90, 0.86);
    let wr_x = g(0.25, 0.20);
    let th_x = g(0.085, 0.088);
    let th_y = g(0.80, 0.72);
    let kn_y = g(0.47, 0.41);
    let kn_x = g(0.085, 0.080);
    let an_y = g(0.08, 0.07);
    let an_x = g(0.085, 0.082);
    let toe_y = 0.02;
    let toe_z = g(0.09, 0.075);

    let mut b = [Vec3::ZERO; BONE_COUNT];
    b[bone::ROOT] = Vec3::ZERO;
    b[bone::HIPS] = Vec3::new(0.0, hips, 0.0);
    b[bone::PELVIS] = Vec3::new(0.0, hips - 0.05, 0.0);
    b[bone::SPINE] = Vec3::new(0.0, spine, 0.0);
    b[bone::CHEST] = Vec3::new(0.0, chest, 0.0);
    b[bone::UPPER_CHEST] = Vec3::new(0.0, upper, 0.0);
    b[bone::NECK] = Vec3::new(0.0, neck, 0.0);
    b[bone::HEAD] = Vec3::new(0.0, head, 0.0);
    for (sign, base) in [(1.0f32, bone::L_CLAVICLE), (-1.0, bone::R_CLAVICLE)] {
        b[base] = Vec3::new(sign * clav_x, clav_y, 0.0);
        b[base + 1] = Vec3::new(sign * sh_x, sh_y, 0.0);
        b[base + 2] = Vec3::new(sign * el_x, el_y, 0.0);
        b[base + 3] = Vec3::new(sign * wr_x, wr_y, 0.0);
    }
    for (sign, base) in [(1.0f32, bone::L_THIGH), (-1.0, bone::R_THIGH)] {
        b[base] = Vec3::new(sign * th_x, th_y, 0.0);
        b[base + 1] = Vec3::new(sign * kn_x, kn_y, 0.0);
        b[base + 2] = Vec3::new(sign * an_x, an_y, 0.0);
        b[base + 3] = Vec3::new(sign * an_x, toe_y, toe_z);
    }
    b
}

/// Rig de posing: posições de bind dos 24 ossos (rotações de bind = identidade) + altura do solado.
#[derive(Debug, Clone, PartialEq)]
pub struct PoseRig {
    /// Posição de mundo do bind de cada osso.
    pub bind: [Vec3; BONE_COUNT],
    /// Altura do solado do pé em repouso (y mínimo da malha).
    pub sole_y: f32,
}

impl Default for PoseRig {
    fn default() -> Self {
        PoseRig::canonical(1.0)
    }
}

/// Número de vértices vizinhos que arrastam cada junta no ajuste.
const FIT_NEIGHBOURS: usize = 24;

impl PoseRig {
    /// Rig canônico interpolado por gênero (`dimorphism`: 0 feminino … 1 masculino).
    pub fn canonical(dimorphism: f32) -> Self {
        let d = if dimorphism.is_finite() { dimorphism.clamp(0.0, 1.0) } else { 1.0 };
        let (m, f) = (canonical_bind_gender(true), canonical_bind_gender(false));
        let mut bind = [Vec3::ZERO; BONE_COUNT];
        for i in 0..BONE_COUNT {
            bind[i] = f[i].lerp(m[i], d);
        }
        PoseRig { bind, sole_y: 0.0 }
    }

    /// Ajusta o rig canônico à malha DEFORMADA: cada junta é deslocada pela média ponderada
    /// (1/d²) dos deslocamentos `deformed − reference` dos `FIT_NEIGHBOURS` vértices mais próximos
    /// dela na malha de referência (gênero interpolado, sem proporções nem morphs). Assim a junta
    /// acompanha proporções e morphs sem tabelas por slider. Malhas incompatíveis devolvem o canônico.
    pub fn fit(reference: &Mesh, deformed: &Mesh, dimorphism: f32) -> Self {
        let mut rig = PoseRig::canonical(dimorphism);
        if reference.vertices.len() != deformed.vertices.len() || reference.vertices.is_empty() {
            return rig;
        }
        let refs: Vec<Vec3> = reference.vertices.iter().map(|v| Vec3::from_array(v.position)).collect();
        let defs: Vec<Vec3> = deformed.vertices.iter().map(|v| Vec3::from_array(v.position)).collect();
        if defs.iter().any(|p| !p.is_finite()) {
            return rig;
        }
        for i in 0..BONE_COUNT {
            if i == bone::ROOT {
                continue;
            }
            let p = rig.bind[i];
            // K menores distâncias (seleção parcial barata: ordena 4k floats)
            let mut best: Vec<(f32, usize)> = Vec::with_capacity(FIT_NEIGHBOURS + 1);
            for (vi, r) in refs.iter().enumerate() {
                let d2 = r.distance_squared(p);
                if best.len() < FIT_NEIGHBOURS || d2 < best[best.len() - 1].0 {
                    let pos = best.partition_point(|(bd, _)| *bd < d2);
                    best.insert(pos, (d2, vi));
                    best.truncate(FIT_NEIGHBOURS);
                }
            }
            let (mut acc, mut wsum) = (Vec3::ZERO, 0.0f32);
            for (d2, vi) in best {
                let w = 1.0 / (d2 + 1e-4);
                acc += (defs[vi] - refs[vi]) * w;
                wsum += w;
            }
            if wsum > 0.0 {
                rig.bind[i] = p + acc / wsum;
            }
        }
        // hierarquia simétrica nos ossos centrais: a malha é simétrica em X, então zera o ruído lateral deles
        for i in [bone::HIPS, bone::PELVIS, bone::SPINE, bone::CHEST, bone::UPPER_CHEST, bone::NECK, bone::HEAD] {
            rig.bind[i].x = 0.0;
        }
        let min_y = defs.iter().map(|p| p.y).fold(f32::INFINITY, f32::min);
        rig.sole_y = if min_y.is_finite() { min_y.min(0.5) } else { 0.0 };
        rig
    }

    /// Offset local de bind do osso em relação ao pai.
    pub fn offset(&self, bone: usize) -> Vec3 {
        match BONE_PARENTS[bone] {
            Some(p) => self.bind[bone] - self.bind[p],
            None => self.bind[bone],
        }
    }

    /// Comprimento do segmento osso→filho principal (para visualização/IK); 0 se for folha.
    pub fn bone_length(&self, bone: usize) -> f32 {
        let tip = match bone {
            bone::HIPS => bone::SPINE,
            bone::L_ANKLE => bone::L_TOES,
            bone::R_ANKLE => bone::R_TOES,
            _ => match bone_children(bone).first() {
                Some(c) => *c,
                None => return 0.0,
            },
        };
        self.bind[tip].distance(self.bind[bone])
    }

    /// Matrizes de skinning (`mundo posado · inverso do bind`) dos 24 ossos.
    ///
    /// `skin[i] = skin[pai] · T(b_i) · R_i · T(−b_i)` com `R_i = T(t_i) · rot_i`. Em repouso é a identidade EXATA.
    pub fn skin_palette(&self, pose: &PoseState) -> [Mat4; BONE_COUNT] {
        let mut skin = [Mat4::IDENTITY; BONE_COUNT];
        for i in 0..BONE_COUNT {
            let q = pose.rotation(i);
            let t = pose.translation(i);
            let local = if q_is_identity(q) && t == Vec3::ZERO {
                Mat4::IDENTITY
            } else {
                let b = self.bind[i];
                Mat4::from_translation(b) * Mat4::from_rotation_translation(q, t) * Mat4::from_translation(-b)
            };
            skin[i] = match BONE_PARENTS[i] {
                Some(p) => skin[p] * local,
                None => local,
            };
        }
        skin
    }

    /// Paleta achatada (16 floats por osso, colunas) no layout do shader.
    pub fn palette_floats(&self, pose: &PoseState) -> Vec<f32> {
        let mut out = Vec::with_capacity(BONE_COUNT * 16);
        for m in self.skin_palette(pose) {
            out.extend_from_slice(&m.to_cols_array());
        }
        out
    }

    /// Posição de mundo posada de cada junta.
    pub fn joint_positions(&self, pose: &PoseState) -> [Vec3; BONE_COUNT] {
        let skin = self.skin_palette(pose);
        let mut out = [Vec3::ZERO; BONE_COUNT];
        for i in 0..BONE_COUNT {
            out[i] = skin[i].transform_point3(self.bind[i]);
        }
        out
    }

    /// Rotação de mundo posada de cada osso (a de bind é a identidade).
    pub fn world_rotations(&self, pose: &PoseState) -> [Quat; BONE_COUNT] {
        let mut out = [Quat::IDENTITY; BONE_COUNT];
        for i in 0..BONE_COUNT {
            let local = pose.rotation(i);
            out[i] = match BONE_PARENTS[i] {
                Some(p) => out[p] * local,
                None => local,
            };
        }
        out
    }

    /// Pontos de contato com o chão (calcanhar e ponta de cada pé) em bind, com o osso que os move.
    pub fn foot_contacts(&self) -> [(usize, Vec3); 6] {
        let heel = |a: usize| Vec3::new(self.bind[a].x, self.sole_y, self.bind[a].z - 0.05);
        let ball = |t: usize| Vec3::new(self.bind[t].x, self.sole_y, self.bind[t].z);
        let tip = |t: usize| Vec3::new(self.bind[t].x, self.sole_y, self.bind[t].z + 0.035);
        [
            (bone::L_ANKLE, heel(bone::L_ANKLE)),
            (bone::L_TOES, ball(bone::L_TOES)),
            (bone::L_TOES, tip(bone::L_TOES)),
            (bone::R_ANKLE, heel(bone::R_ANKLE)),
            (bone::R_TOES, ball(bone::R_TOES)),
            (bone::R_TOES, tip(bone::R_TOES)),
        ]
    }

    /// Pontos de contato posados (mundo).
    pub fn posed_foot_contacts(&self, pose: &PoseState) -> [(usize, Vec3); 6] {
        let skin = self.skin_palette(pose);
        let mut c = self.foot_contacts();
        for (b, p) in c.iter_mut() {
            *p = skin[*b].transform_point3(*p);
        }
        c
    }

    /// Menor altura entre os pontos de contato (negativo = pé afundado no chão).
    pub fn lowest_contact_y(&self, pose: &PoseState) -> f32 {
        self.posed_foot_contacts(pose).iter().map(|(_, p)| p.y).fold(f32::INFINITY, f32::min)
    }

    /// Pose com `root_translation.y` ajustado para o ponto de contato mais baixo tocar y = `floor`.
    pub fn grounded(&self, pose: &PoseState, floor: f32) -> PoseState {
        let mut out = pose.clone();
        let low = self.lowest_contact_y(pose);
        out.root_translation[1] += floor - low;
        if out.root_translation[1].abs() < 1e-6 {
            out.root_translation[1] = 0.0;
        }
        out
    }

    /// Centro de massa (mundo) pelas frações de massa de segmento (tipo Dempster, [S/C]).
    pub fn center_of_mass(&self, pose: &PoseState) -> Vec3 {
        use bone::*;
        let j = self.joint_positions(pose);
        let mid = |a: usize, b: usize| (j[a] + j[b]) * 0.5;
        let hand_tip = |w: usize, e: usize| j[w] + (j[w] - j[e]).normalize_or_zero() * 0.07;
        let head_c = j[HEAD] + (j[HEAD] - j[NECK]).normalize_or_zero() * 0.11;
        let segs: [(f32, Vec3); 15] = [
            (0.500, mid(HIPS, UPPER_CHEST)),
            (0.081, (j[NECK] + head_c) * 0.5),
            (0.028, mid(L_SHOULDER, L_ELBOW)),
            (0.028, mid(R_SHOULDER, R_ELBOW)),
            (0.016, mid(L_ELBOW, L_WRIST)),
            (0.016, mid(R_ELBOW, R_WRIST)),
            (0.006, (j[L_WRIST] + hand_tip(L_WRIST, L_ELBOW)) * 0.5),
            (0.006, (j[R_WRIST] + hand_tip(R_WRIST, R_ELBOW)) * 0.5),
            (0.100, mid(L_THIGH, L_KNEE)),
            (0.100, mid(R_THIGH, R_KNEE)),
            (0.0465, mid(L_KNEE, L_ANKLE)),
            (0.0465, mid(R_KNEE, R_ANKLE)),
            (0.0145, mid(L_ANKLE, L_TOES)),
            (0.0145, mid(R_ANKLE, R_TOES)),
            (0.0, Vec3::ZERO),
        ];
        let total: f32 = segs.iter().map(|s| s.0).sum();
        let sum = segs.iter().fold(Vec3::ZERO, |a, s| a + s.1 * s.0);
        sum / total
    }
}

/// Envoltória convexa 2D (x, z) — polígono de suporte. Pontos em ordem anti-horária; 0..2 pontos devolvem os próprios.
pub fn convex_hull_xz(points: &[Vec3]) -> Vec<[f32; 2]> {
    let mut pts: Vec<[f32; 2]> = points.iter().map(|p| [p.x, p.z]).filter(|p| p[0].is_finite() && p[1].is_finite()).collect();
    pts.sort_by(|a, b| a[0].partial_cmp(&b[0]).unwrap_or(std::cmp::Ordering::Equal).then(a[1].partial_cmp(&b[1]).unwrap_or(std::cmp::Ordering::Equal)));
    pts.dedup();
    if pts.len() < 3 {
        return pts;
    }
    let cross = |o: [f32; 2], a: [f32; 2], b: [f32; 2]| (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0]);
    let mut lower: Vec<[f32; 2]> = Vec::new();
    for &p in &pts {
        while lower.len() >= 2 && cross(lower[lower.len() - 2], lower[lower.len() - 1], p) <= 0.0 {
            lower.pop();
        }
        lower.push(p);
    }
    let mut upper: Vec<[f32; 2]> = Vec::new();
    for &p in pts.iter().rev() {
        while upper.len() >= 2 && cross(upper[upper.len() - 2], upper[upper.len() - 1], p) <= 0.0 {
            upper.pop();
        }
        upper.push(p);
    }
    lower.pop();
    upper.pop();
    lower.extend(upper);
    lower
}

/// Ponto dentro (ou na borda) do polígono convexo anti-horário.
pub fn point_in_hull(hull: &[[f32; 2]], p: [f32; 2]) -> bool {
    match hull.len() {
        0 => false,
        1 => (hull[0][0] - p[0]).abs() < 1e-4 && (hull[0][1] - p[1]).abs() < 1e-4,
        2 => {
            let (a, b) = (hull[0], hull[1]);
            let ab = [b[0] - a[0], b[1] - a[1]];
            let cross = ab[0] * (p[1] - a[1]) - ab[1] * (p[0] - a[0]);
            let len2 = ab[0] * ab[0] + ab[1] * ab[1];
            let t = if len2 > 0.0 { ((p[0] - a[0]) * ab[0] + (p[1] - a[1]) * ab[1]) / len2 } else { 0.0 };
            cross.abs() < 1e-3 && (-0.01..=1.01).contains(&t)
        }
        n => (0..n).all(|i| {
            let (a, b) = (hull[i], hull[(i + 1) % n]);
            (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0]) >= -1e-5
        }),
    }
}

/// Balanço: `(centro de massa projetado, polígono de suporte, dentro?)`. Só considera pés próximos do chão
/// (`tolerance` m acima do ponto mais baixo); sem pés apoiados devolve `dentro = false`.
pub fn balance(rig: &PoseRig, pose: &PoseState, tolerance: f32) -> (Vec3, Vec<[f32; 2]>, bool) {
    let com = rig.center_of_mass(pose);
    let contacts = rig.posed_foot_contacts(pose);
    let low = contacts.iter().map(|(_, p)| p.y).fold(f32::INFINITY, f32::min);
    let support: Vec<Vec3> = contacts.iter().filter(|(_, p)| p.y <= low + tolerance).map(|(_, p)| *p).collect();
    let hull = convex_hull_xz(&support);
    let inside = point_in_hull(&hull, [com.x, com.z]);
    (com, hull, inside)
}

// ---------------------------------------------------------------------------------------------
// Ativo de pose (.anigopose.json)
// ---------------------------------------------------------------------------------------------

/// Sufixo do arquivo de pose da Biblioteca.
pub const POSE_ASSET_SUFFIX: &str = ".anigopose.json";
/// Identificador do formato.
pub const POSE_ASSET_FORMAT: &str = "anigo.pose";
/// Versão do formato.
pub const POSE_ASSET_VERSION: u32 = 1;
/// Tamanho máximo aceito de um arquivo de pose.
pub const POSE_ASSET_MAX_BYTES: usize = 256 * 1024;

/// Formato de arquivo de uma pose (`.anigopose.json`): por NOME de osso, com máscara.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PoseAsset {
    pub format: String,
    pub version: u32,
    pub name: String,
    /// Ossos que o ativo contém (os demais NÃO são tocados ao aplicar). Vazio = todos.
    #[serde(default)]
    pub bones: Vec<String>,
    pub pose: PoseState,
    /// Proporção do rig de origem (altura da junta Head, para diagnóstico/retarget futuro).
    #[serde(default)]
    pub rig_hint: Option<RigHint>,
    #[serde(default)]
    pub tags: Vec<String>,
}

/// Dica do rig de origem do ativo.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RigHint {
    pub head_height: f32,
    pub hips_height: f32,
}

/// Erros do formato de pose.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum PoseAssetError {
    #[error("arquivo de pose grande demais ({0} bytes)")]
    TooLarge(usize),
    #[error("JSON de pose inválido: {0}")]
    Json(String),
    #[error("formato desconhecido '{0}' (esperado '{POSE_ASSET_FORMAT}')")]
    Format(String),
    #[error("versão {0} do formato de pose não suportada (máximo {POSE_ASSET_VERSION})")]
    Version(u32),
    #[error("pose inválida: {0}")]
    Invalid(String),
}

impl PoseAsset {
    /// Ativo com TODOS os ossos da pose.
    pub fn new(name: impl Into<String>, pose: PoseState) -> Self {
        PoseAsset { format: POSE_ASSET_FORMAT.into(), version: POSE_ASSET_VERSION, name: name.into(), bones: Vec::new(), pose, rig_hint: None, tags: Vec::new() }
    }

    /// Ativo só com os ossos de `mask` (como o "create pose asset" do Blender com ossos selecionados).
    pub fn from_mask(name: impl Into<String>, pose: &PoseState, mask: &BTreeSet<usize>, rig: &PoseRig) -> Self {
        let mut a = PoseAsset::new(name, pose.masked(mask));
        a.bones = mask.iter().map(|i| BONE_NAMES[*i].to_string()).collect();
        a.rig_hint = Some(RigHint { head_height: rig.bind[bone::HEAD].y, hips_height: rig.bind[bone::HIPS].y });
        a
    }

    /// Máscara efetiva (índices). Vazia no arquivo = todos os ossos.
    pub fn mask(&self) -> BTreeSet<usize> {
        if self.bones.is_empty() {
            (0..BONE_COUNT).collect()
        } else {
            self.bones.iter().filter_map(|n| bone_index(n)).collect()
        }
    }

    pub fn to_json(&self) -> Result<String, PoseAssetError> {
        serde_json::to_string_pretty(self).map_err(|e| PoseAssetError::Json(e.to_string()))
    }

    /// Lê e valida (`format`, `version`, nomes de osso, finitude, tamanho).
    pub fn from_json(raw: &[u8]) -> Result<PoseAsset, PoseAssetError> {
        if raw.len() > POSE_ASSET_MAX_BYTES {
            return Err(PoseAssetError::TooLarge(raw.len()));
        }
        let mut a: PoseAsset = serde_json::from_slice(raw).map_err(|e| PoseAssetError::Json(e.to_string()))?;
        if a.format != POSE_ASSET_FORMAT {
            return Err(PoseAssetError::Format(a.format));
        }
        if a.version == 0 || a.version > POSE_ASSET_VERSION {
            return Err(PoseAssetError::Version(a.version));
        }
        for n in &a.bones {
            if bone_index(n).is_none() {
                return Err(PoseAssetError::Invalid(format!("osso desconhecido '{n}'")));
            }
        }
        a.pose.validate().map_err(PoseAssetError::Invalid)?;
        a.pose.sanitize();
        Ok(a)
    }

    /// Resultado de aplicar o ativo à pose `current`: `blend` em [0, 1+] (1 = totalmente), `flipped` espelha,
    /// `only` restringe a ossos selecionados (interseção com a máscara do ativo).
    pub fn apply_to(&self, current: &PoseState, blend: f32, flipped: bool, only: Option<&BTreeSet<usize>>) -> PoseState {
        let mut asset_pose = if flipped { self.pose.mirrored() } else { self.pose.clone() };
        let mut mask: BTreeSet<usize> = if flipped { self.mask().into_iter().map(mirror_bone).collect() } else { self.mask() };
        if let Some(sel) = only {
            mask = mask.intersection(sel).copied().collect();
        }
        asset_pose = asset_pose.masked(&mask);
        let target = current.overlay(&asset_pose, &mask);
        if (blend - 1.0).abs() < 1e-6 {
            target
        } else {
            PoseState::blend(current, &target, blend)
        }
    }
}

/// Atalho: ativo com a pose inteira.
pub fn pose_to_asset(name: &str, pose: &PoseState) -> PoseAsset {
    PoseAsset::new(name, pose.clone())
}

// ---------------------------------------------------------------------------------------------
// Dedos: deformação CPU da malha canônica
// ---------------------------------------------------------------------------------------------

/// Vértices de dedo da malha canônica: o gerador emite, depois das mãos, 5 dedos por mão (4 anéis × 6 setores →
/// 5 × 7 = 35 vértices por dedo), primeiro o lado "esquerdo do gerador" (x < 0, osso direito do personagem) e
/// depois o outro, dedo a dedo (ver `Mesh::create_canonical_base_unskinned`).
pub const FINGER_VERTS: usize = 35;
const FINGER_COUNT: usize = 5;
const FINGER_BLOCK: usize = FINGER_VERTS * FINGER_COUNT * 2;

/// Índice do primeiro vértice dos dedos na malha canônica (depois de cabeça, pescoço, tronco, pelve,
/// deltoides, braços, antebraços e mãos).
pub const FINGERS_START: usize = 425 + 119 + 525 + 375 + 234 + 234 + 234 + 182;

/// Curva os dedos da malha CANÔNICA em `mesh` (CPU, antes do skinning; não altera a paleta nem o contrato).
/// Cada dedo é um cilindro que pende do pulso: gira em torno do eixo X do nó do dedo por `curl · 100°`,
/// com mais curvatura na ponta (3 falanges aproximadas pelos 4 anéis). Retorna `false` se a malha não for a canônica.
pub fn finger_curl_mesh(mesh: &mut Mesh, hands: &HandsPose) -> bool {
    if hands.is_rest() {
        return true;
    }
    if mesh.vertices.len() < FINGERS_START + FINGER_BLOCK {
        return false;
    }
    // Ordem de emissão: para cada dedo f (0..5): primeiro o par "x<0" depois "x>0" (add_limb_pair).
    for finger in 0..FINGER_COUNT {
        for side in 0..2 {
            // `add_limb_pair` emite (esquerda do gerador = x<0) e depois (direita = x>0) por dedo
            let start = FINGERS_START + (finger * 2 + side) * FINGER_VERTS;
            let right_of_character = side == 0; // x < 0 → osso direito do personagem
            let curl = if right_of_character { hands.right[finger] } else { hands.left[finger] };
            if curl <= 1e-6 {
                continue;
            }
            // 5 anéis (rings+1 = 5) de 7 vértices (sectors+1 = 7)
            let ring = |r: usize| start + r * 7;
            let base_ctr = {
                let mut c = [0.0f32; 3];
                for s in 0..7 {
                    for k in 0..3 {
                        c[k] += mesh.vertices[ring(0) + s].position[k] / 7.0;
                    }
                }
                c
            };
            for r in 0..5 {
                // ângulo acumulado até o anel r (articulado: nó em r = 1, 2, 3)
                let t = r as f32 / 4.0;
                let angle = curl * 100.0_f32.to_radians() * t.powf(1.2);
                let (s, c) = angle.sin_cos();
                let s = -s; // curva para a frente (+Z)
                for sct in 0..7 {
                    let v = &mut mesh.vertices[ring(r) + sct];
                    // gira em torno do eixo X que passa por (base_ctr.y, base_ctr.z): o dedo pende para −Y e curva para +Z
                    let dy = v.position[1] - base_ctr[1];
                    let dz = v.position[2] - base_ctr[2];
                    v.position[1] = base_ctr[1] + dy * c - dz * s;
                    v.position[2] = base_ctr[2] + dy * s + dz * c;
                    let (ny, nz) = (v.normal[1], v.normal[2]);
                    v.normal[1] = ny * c - nz * s;
                    v.normal[2] = ny * s + nz * c;
                }
            }
        }
    }
    true
}

// ---------------------------------------------------------------------------------------------
// Presets
// ---------------------------------------------------------------------------------------------

/// Presets de corpo e de mão. Todos devolvem poses COMPLETAS (rotações absolutas sobre o repouso).
pub mod presets {
    use super::*;

    /// Chaves estáveis dos presets de corpo.
    pub const BODY: [&str; 6] = ["t", "a", "standing", "sitting", "walking", "action"];
    /// Chaves estáveis dos presets de mão.
    pub const HAND: [&str; 6] = ["open", "fist", "point", "v", "hold", "relaxed"];

    fn rx(d: f32) -> Quat {
        Quat::from_rotation_x(d.to_radians())
    }
    fn ry(d: f32) -> Quat {
        Quat::from_rotation_y(d.to_radians())
    }
    fn rz(d: f32) -> Quat {
        Quat::from_rotation_z(d.to_radians())
    }

    /// Aplica a rotação nos dois lados de forma simétrica (`left` descreve o lado esquerdo; o direito é o espelho).
    fn sym(pose: &mut PoseState, left_bone: usize, left: Quat) {
        pose.set_rotation(left_bone, left);
        let m = Quat::from_xyzw(left.x, -left.y, -left.z, left.w);
        pose.set_rotation(mirror_bone(left_bone), m);
    }

    /// Preset de corpo por chave; `None` para chave desconhecida. `rig` serve ao apoio dos pés.
    pub fn body(key: &str, rig: &PoseRig) -> Option<PoseState> {
        let mut p = PoseState::default();
        match key {
            "t" => {
                sym(&mut p, bone::L_SHOULDER, rz(90.0));
            }
            "a" => {
                sym(&mut p, bone::L_SHOULDER, rz(40.0));
                sym(&mut p, bone::L_ELBOW, rx(-8.0));
            }
            "standing" => {
                // em pé relaxado, peso levemente num dos pés
                sym(&mut p, bone::L_SHOULDER, rz(8.0));
                sym(&mut p, bone::L_ELBOW, rx(-12.0));
                p.set_rotation(bone::SPINE, rz(-2.0));
                p.set_rotation(bone::HEAD, ry(4.0));
                p.set_rotation(bone::L_THIGH, rz(-3.0));
                p.set_rotation(bone::R_THIGH, rz(2.0));
            }
            "sitting" => {
                sym(&mut p, bone::L_THIGH, rx(-90.0) * rz(4.0));
                sym(&mut p, bone::L_KNEE, rx(90.0));
                sym(&mut p, bone::L_ANKLE, rx(0.0));
                sym(&mut p, bone::L_SHOULDER, rx(-25.0) * rz(8.0));
                sym(&mut p, bone::L_ELBOW, rx(-70.0));
                p.set_rotation(bone::SPINE, rx(8.0));
                p.set_rotation(bone::CHEST, rx(6.0));
                p.set_rotation(bone::HEAD, rx(-8.0));
                return Some(rig.grounded(&p, 0.0));
            }
            "walking" => {
                // contato: perna esquerda à frente, direita atrás; braços em contra-balanço
                p.set_rotation(bone::L_THIGH, rx(-28.0));
                p.set_rotation(bone::L_KNEE, rx(6.0));
                p.set_rotation(bone::L_ANKLE, rx(-10.0));
                p.set_rotation(bone::R_THIGH, rx(18.0));
                p.set_rotation(bone::R_KNEE, rx(28.0));
                p.set_rotation(bone::R_ANKLE, rx(30.0));
                p.set_rotation(bone::R_TOES, rx(25.0));
                p.set_rotation(bone::L_SHOULDER, rx(22.0) * rz(8.0));
                p.set_rotation(bone::L_ELBOW, rx(-20.0));
                p.set_rotation(bone::R_SHOULDER, rx(-28.0) * rz(-8.0));
                p.set_rotation(bone::R_ELBOW, rx(-35.0));
                p.set_rotation(bone::SPINE, ry(-6.0));
                p.set_rotation(bone::CHEST, ry(-6.0));
                p.set_rotation(bone::HIPS, ry(8.0));
                p.set_rotation(bone::HEAD, ry(-6.0));
                return Some(rig.grounded(&p, 0.0));
            }
            "action" => {
                // postura de combate: pernas abertas e flexionadas, tronco inclinado, soco à frente
                p.set_rotation(bone::HIPS, ry(-30.0));
                p.set_rotation(bone::L_THIGH, rx(-55.0) * rz(30.0));
                p.set_rotation(bone::L_KNEE, rx(70.0));
                p.set_rotation(bone::L_ANKLE, rx(-5.0));
                p.set_rotation(bone::R_THIGH, rx(20.0) * rz(-22.0));
                p.set_rotation(bone::R_KNEE, rx(40.0));
                p.set_rotation(bone::R_ANKLE, rx(10.0));
                p.set_rotation(bone::SPINE, rx(10.0) * ry(12.0));
                p.set_rotation(bone::CHEST, rx(8.0) * ry(12.0));
                p.set_rotation(bone::UPPER_CHEST, ry(10.0));
                p.set_rotation(bone::NECK, ry(-20.0));
                p.set_rotation(bone::HEAD, rx(-10.0) * ry(-18.0));
                p.set_rotation(bone::L_SHOULDER, rx(-85.0) * rz(8.0));
                p.set_rotation(bone::L_ELBOW, rx(-8.0));
                p.set_rotation(bone::R_SHOULDER, rx(-40.0) * rz(-30.0));
                p.set_rotation(bone::R_ELBOW, rx(-115.0));
                p.hands.left = [0.9, 1.0, 1.0, 1.0, 1.0];
                p.hands.right = [0.8, 1.0, 1.0, 1.0, 1.0];
                return Some(rig.grounded(&p, 0.0));
            }
            _ => return None,
        }
        Some(rig.grounded(&p, 0.0))
    }

    /// Curvaturas de dedos de um preset de mão.
    pub fn hand(key: &str) -> Option<FingerCurls> {
        Some(match key {
            "open" => [0.0, 0.0, 0.0, 0.0, 0.0],
            "relaxed" => [0.25, 0.3, 0.35, 0.4, 0.45],
            "fist" => [0.8, 1.0, 1.0, 1.0, 1.0],
            "point" => [0.7, 0.0, 1.0, 1.0, 1.0],
            "v" => [0.7, 0.0, 0.0, 1.0, 1.0],
            "hold" => [0.5, 0.65, 0.65, 0.65, 0.65],
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests;
