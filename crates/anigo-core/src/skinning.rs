//! ANIGO — atribuição canônica de ossos (P1 — Robustez, item 4: skinning real).
//!
//! Sem isto o atributo de skin do vértice (`joints`/`weights`, 16 B dos 72 B do
//! vértice) nunca significava nada: a paleta de 24 ossos não tinha em quem
//! aplicar, então nenhum shader podia deformar por osso.
//!
//! ## Duas numerações de osso convivem aqui
//!
//! - **Legada (`legacy`)**: a que a própria malha canônica carrega
//!   (`mesh.rs::create_canonical_base` marca cada parte do corpo com 0..18).
//!   Ela diz **qual parte do corpo** o vértice é (pelve, tronco, pescoço,
//!   cabeça, deltoide, braço, antebraço, mão, coxa, joelho/perna, pé).
//! - **Canônica (`joints`)**: a ordem VRM 1.0 do `BondSyncManager` (24 ossos),
//!   a única que a paleta de skinning conhece.
//!
//! O lado do corpo **não** vem da numeração legada: `mesh.rs` gera
//! `joint_left` em `x < 0` e `joint_right` em `x > 0`, o **espelho** do
//! esqueleto canônico (`LeftClavicle` em `+X`). O lado é decidido pela
//! geometria (`x >= 0` ⇒ cadeia `LEFT_*`), e há teste fixando as duas
//! convenções — um lado trocado aqui giraria o braço em torno do ombro oposto.
//!
//! ## Neutro por construção
//!
//! Enquanto [`PROPORTIONS_BAKED_INTO_BASE_MESH`] for `true`, o núcleo assa as
//! proporções na malha base e entrega a paleta **identidade**: skinning e bake
//! não podem aplicar a mesma deformação duas vezes. Com a paleta neutra, a
//! atribuição é exatamente neutra — `Σ wᵢ · (I · p) = p` (ver `tests`), então
//! os shaders podem aplicar LBS desde já sem mudar um pixel.

use crate::bone_sync::CANONICAL_JOINT_COUNT;
use crate::mesh::Mesh;

/// Influências por vértice suportadas pelo layout de vértice (72 B).
pub const MAX_INFLUENCES: usize = 4;

/// As proporções são assadas na malha base por `prepare_base_mesh`?
///
/// `true` hoje: o núcleo dobra gênero/proporções na geometria base (e os morphs
/// vêm em delta). Quando o rig passar a dirigir as proporções (P2), esta
/// constante vira `false`, o bake sai de `prepare_base_mesh` e a paleta deixa de
/// ser neutra — a malha, os morphs e a paleta já estão prontos para isso.
pub const PROPORTIONS_BAKED_INTO_BASE_MESH: bool = true;

/// Ossos do esqueleto canônico (ordem do `BondSyncManager`, VRM 1.0).
pub mod joints {
    pub const ROOT: u32 = 0;
    pub const HIPS: u32 = 1;
    pub const PELVIS: u32 = 2;
    pub const SPINE: u32 = 3;
    pub const CHEST: u32 = 4;
    pub const UPPER_CHEST: u32 = 5;
    pub const NECK: u32 = 6;
    pub const HEAD: u32 = 7;
    pub const LEFT_CLAVICLE: u32 = 8;
    pub const LEFT_SHOULDER: u32 = 9;
    pub const LEFT_ELBOW: u32 = 10;
    pub const LEFT_WRIST: u32 = 11;
    pub const RIGHT_CLAVICLE: u32 = 12;
    pub const RIGHT_SHOULDER: u32 = 13;
    pub const RIGHT_ELBOW: u32 = 14;
    pub const RIGHT_WRIST: u32 = 15;
    pub const LEFT_THIGH: u32 = 16;
    pub const LEFT_KNEE: u32 = 17;
    pub const LEFT_ANKLE: u32 = 18;
    pub const LEFT_TOES: u32 = 19;
    pub const RIGHT_THIGH: u32 = 20;
    pub const RIGHT_KNEE: u32 = 21;
    pub const RIGHT_ANKLE: u32 = 22;
    pub const RIGHT_TOES: u32 = 23;
}

/// Numeração **legada** gravada pela malha canônica (0..18).
///
/// É a única fonte de "qual parte do corpo" — não inferimos partes por faixa de
/// índice nem por posição, para que a atribuição sobreviva a mudanças na
/// contagem de vértices.
pub mod legacy {
    pub const PELVIS: u16 = 0;
    /// Reservado: o gerador **não** emite este id (o tronco começa em
    /// [`TORSO`]); existe para a tabela ficar completa e para um id 1 vindo de
    /// fora cair na espinha em vez de virar osso errado.
    pub const SPINE: u16 = 1;
    pub const TORSO: u16 = 2;
    pub const NECK: u16 = 3;
    pub const HEAD: u16 = 4;
    /// Deltoides/ombros: 5 = par "esquerdo" do gerador (`x < 0`), 9 = `x > 0`.
    pub const DELTOID_LEFT: u16 = 5;
    pub const UPPER_ARM_LEFT: u16 = 6;
    pub const FOREARM_LEFT: u16 = 7;
    pub const HAND_LEFT: u16 = 8;
    pub const DELTOID_RIGHT: u16 = 9;
    pub const UPPER_ARM_RIGHT: u16 = 10;
    pub const FOREARM_RIGHT: u16 = 11;
    pub const HAND_RIGHT: u16 = 12;
    pub const THIGH_LEFT: u16 = 13;
    pub const KNEE_CALF_LEFT: u16 = 14;
    pub const FOOT_LEFT: u16 = 15;
    pub const THIGH_RIGHT: u16 = 16;
    pub const KNEE_CALF_RIGHT: u16 = 17;
    pub const FOOT_RIGHT: u16 = 18;
}

/// Parte do corpo de um vértice (o que a numeração legada realmente diz).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyPart {
    Pelvis,
    Spine,
    Torso,
    Neck,
    Head,
    Deltoid,
    UpperArm,
    Forearm,
    Hand,
    Thigh,
    KneeCalf,
    Foot,
}

impl BodyPart {
    /// Eixo usado para a transição dentro da parte (0 = X, 1 = Y, 2 = Z).
    pub fn blend_axis(self) -> usize {
        match self {
            BodyPart::Deltoid | BodyPart::UpperArm | BodyPart::Forearm | BodyPart::Hand => 0,
            BodyPart::Foot => 2,
            _ => 1,
        }
    }

    /// A transição usa `|x|` (braços têm os dois lados no mesmo eixo).
    pub fn uses_absolute_axis(self) -> bool {
        self.blend_axis() == 0
    }

    pub fn name(self) -> &'static str {
        match self {
            BodyPart::Pelvis => "pelvis",
            BodyPart::Spine => "spine",
            BodyPart::Torso => "torso",
            BodyPart::Neck => "neck",
            BodyPart::Head => "head",
            BodyPart::Deltoid => "deltoid",
            BodyPart::UpperArm => "upper_arm",
            BodyPart::Forearm => "forearm",
            BodyPart::Hand => "hand",
            BodyPart::Thigh => "thigh",
            BodyPart::KneeCalf => "knee_calf",
            BodyPart::Foot => "foot",
        }
    }
}

/// Todas as partes, na ordem em que os limites são medidos.
pub const BODY_PARTS: [BodyPart; 12] = [
    BodyPart::Pelvis,
    BodyPart::Spine,
    BodyPart::Torso,
    BodyPart::Neck,
    BodyPart::Head,
    BodyPart::Deltoid,
    BodyPart::UpperArm,
    BodyPart::Forearm,
    BodyPart::Hand,
    BodyPart::Thigh,
    BodyPart::KneeCalf,
    BodyPart::Foot,
];

/// Parte do corpo de um id legado (`None` quando o id não é canônico).
pub fn part_of(legacy_id: u16) -> Option<BodyPart> {
    Some(match legacy_id {
        legacy::PELVIS => BodyPart::Pelvis,
        legacy::SPINE => BodyPart::Spine,
        legacy::TORSO => BodyPart::Torso,
        legacy::NECK => BodyPart::Neck,
        legacy::HEAD => BodyPart::Head,
        legacy::DELTOID_LEFT | legacy::DELTOID_RIGHT => BodyPart::Deltoid,
        legacy::UPPER_ARM_LEFT | legacy::UPPER_ARM_RIGHT => BodyPart::UpperArm,
        legacy::FOREARM_LEFT | legacy::FOREARM_RIGHT => BodyPart::Forearm,
        legacy::HAND_LEFT | legacy::HAND_RIGHT => BodyPart::Hand,
        legacy::THIGH_LEFT | legacy::THIGH_RIGHT => BodyPart::Thigh,
        legacy::KNEE_CALF_LEFT | legacy::KNEE_CALF_RIGHT => BodyPart::KneeCalf,
        legacy::FOOT_LEFT | legacy::FOOT_RIGHT => BodyPart::Foot,
        _ => return None,
    })
}

/// Lado do corpo na convenção do **esqueleto canônico** (`LeftClavicle` em +X).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoneSide {
    Left,
    Right,
}

impl BoneSide {
    /// O esqueleto canônico tem a esquerda em `x >= 0` (VRM 1.0).
    pub fn of(x: f32) -> Self {
        if x >= 0.0 {
            BoneSide::Left
        } else {
            BoneSide::Right
        }
    }

    /// Par `(esquerda, direita)` de um osso lateral.
    pub fn pick(self, left: u32, right: u32) -> u32 {
        match self {
            BoneSide::Left => left,
            BoneSide::Right => right,
        }
    }
}

/// Resultado da atribuição (contagens para diagnóstico/telemetria/testes).
#[derive(Debug, Clone, PartialEq)]
pub struct SkinAssignmentSummary {
    /// Vértices com pelo menos um osso e peso não nulo.
    pub assigned: usize,
    /// Total de vértices vistos.
    pub total: usize,
    /// Ossos canônicos realmente usados.
    pub joints_used: Vec<u32>,
    /// Partes do corpo que receberam vértices.
    pub parts_used: Vec<&'static str>,
    /// Maior número de influências em um vértice (≤ [`MAX_INFLUENCES`]).
    pub max_influences: usize,
    /// Vértices com id legado fora da numeração canônica (não deveria haver).
    pub unmapped_vertices: usize,
    /// Convenção de lado aplicada (esqueleto canônico: esquerda em +X).
    pub left_is_positive_x: bool,
    /// Todos os vértices com pesos somando 1 (tolerância de 1e-5).
    pub weights_normalized: bool,
}

impl SkinAssignmentSummary {
    /// A atribuição cobre a malha inteira (nenhum vértice solto).
    pub fn is_complete(&self) -> bool {
        self.assigned == self.total && self.total > 0 && self.unmapped_vertices == 0
    }
}

/// Limites `[min, max]` por eixo, por parte do corpo (medidos na malha).
#[derive(Debug, Clone, PartialEq)]
pub struct PartBounds {
    bounds: Vec<[[f32; 2]; 3]>,
}

impl PartBounds {
    /// Mede a extensão de cada parte na malha (base da transição suave).
    ///
    /// A extensão sai das posições dos próprios vértices, então a atribuição
    /// acompanha a malha se a escala mudar — nenhum limite hard-coded.
    pub fn measure(mesh: &Mesh) -> Self {
        let mut bounds = vec![[[f32::INFINITY, f32::NEG_INFINITY]; 3]; BODY_PARTS.len()];
        for vertex in &mesh.vertices {
            let Some(part) = part_of(vertex.joints[0]) else {
                continue;
            };
            let index = part_index(part);
            for axis in 0..3 {
                let mut value = vertex.position[axis];
                if part.uses_absolute_axis() && axis == part.blend_axis() {
                    value = value.abs();
                }
                bounds[index][axis][0] = bounds[index][axis][0].min(value);
                bounds[index][axis][1] = bounds[index][axis][1].max(value);
            }
        }
        for entry in bounds.iter_mut() {
            for axis in entry.iter_mut() {
                if !axis[0].is_finite() || !axis[1].is_finite() || axis[0] > axis[1] {
                    *axis = [0.0, 1.0];
                }
            }
        }
        Self { bounds }
    }

    /// Limites neutros (0..1 em todos os eixos) — transição ainda utilizável.
    pub fn neutral() -> Self {
        Self {
            bounds: vec![[[0.0, 1.0]; 3]; BODY_PARTS.len()],
        }
    }

    /// Fração 0..1 da posição dentro da extensão da parte (no eixo de transição).
    pub fn fraction(&self, part: BodyPart, position: [f32; 3]) -> f32 {
        let axis = part.blend_axis();
        let mut value = position[axis];
        if part.uses_absolute_axis() {
            value = value.abs();
        }
        let [min, max] = self.bounds[part_index(part)][axis];
        if (max - min).abs() < 1e-6 {
            return 0.5;
        }
        ((value - min) / (max - min)).clamp(0.0, 1.0)
    }
}

impl Default for PartBounds {
    fn default() -> Self {
        Self::neutral()
    }
}

fn part_index(part: BodyPart) -> usize {
    BODY_PARTS
        .iter()
        .position(|candidate| *candidate == part)
        .unwrap_or(0)
}

/// Distribui os vértices da malha nos ossos canônicos.
///
/// `legacy_ids` é a numeração legada **explícita** (uma por vértice), lida antes
/// de qualquer sobrescrita. Ela é entrada obrigatória de propósito: as duas
/// numerações se sobrepõem em 0..18, então "ler os ids atuais" depois de uma
/// primeira passada seria ambíguo e erraria ossos em silêncio — aqui o
/// sobrescrito é impossível de confundir com uma leitura.
///
/// Idempotente **para a mesma entrada**: chamar de novo com os mesmos
/// `legacy_ids` dá exatamente o mesmo resultado (há teste).
pub fn assign_canonical_skin_weights(
    mesh: &mut Mesh,
    legacy_ids: &[u16],
) -> SkinAssignmentSummary {
    let bounds = PartBounds::measure(mesh);
    assign_with_bounds(mesh, legacy_ids, &bounds)
}

/// Conveniência para a malha recém-gerada: os ids atuais **são** os legados.
///
/// Só vale para uma malha saída de `Mesh::create_canonical_base` (antes de
/// qualquer atribuição canônica).
pub fn assign_legacy_skin_weights(mesh: &mut Mesh) -> SkinAssignmentSummary {
    let legacy_ids: Vec<u16> = mesh
        .vertices
        .iter()
        .map(|vertex| vertex.joints[0])
        .collect();
    assign_canonical_skin_weights(mesh, &legacy_ids)
}

fn assign_with_bounds(
    mesh: &mut Mesh,
    legacy_ids: &[u16],
    bounds: &PartBounds,
) -> SkinAssignmentSummary {
    let total = mesh.vertices.len();
    let mut joints_used: Vec<u32> = Vec::new();
    let mut parts_used: Vec<&'static str> = Vec::new();
    let mut max_influences = 0usize;
    let mut unmapped_vertices = 0usize;
    let mut weights_normalized = true;

    for (index, vertex) in mesh.vertices.iter_mut().enumerate() {
        // Id ausente (slice curto) conta como não mapeado — nunca vira osso 0.
        let legacy_id = legacy_ids.get(index).copied().unwrap_or(u16::MAX);
        let (joints, weights) = skin_for_vertex(legacy_id, vertex.position, bounds);
        vertex.joints = joints;
        vertex.weights = weights;

        let influences = weights.iter().filter(|weight| **weight > 0.0).count();
        max_influences = max_influences.max(influences);
        let sum: f32 = weights.iter().sum();
        if (sum - 1.0).abs() > 1e-5 {
            weights_normalized = false;
        }

        match part_of(legacy_id) {
            Some(part) => {
                if !parts_used.contains(&part.name()) {
                    parts_used.push(part.name());
                }
            }
            None => unmapped_vertices += 1,
        }
        // `joints` é `[u16; 4]` (layout do vértice); o resumo publica u32 para
        // não depender da largura do atributo na GPU.
        for joint in joints.iter().take(influences) {
            let joint = u32::from(*joint);
            if !joints_used.contains(&joint) {
                joints_used.push(joint);
            }
        }
    }

    joints_used.sort_unstable();
    parts_used.sort_unstable();
    SkinAssignmentSummary {
        assigned: total.saturating_sub(unmapped_vertices),
        total,
        joints_used,
        parts_used,
        max_influences,
        unmapped_vertices,
        left_is_positive_x: true,
        weights_normalized,
    }
}

/// Ossos (até dois) de um vértice, com os pesos somando 1.
///
/// `legacy_id` é a parte do corpo gravada pela malha canônica; o lado sai da
/// geometria (esqueleto canônico: esquerda em `+X`). Um id fora da numeração
/// canônica cai no quadril (osso 1) — nunca em `joints = 0` (raiz sem massa).
pub fn skin_for_vertex(
    legacy_id: u16,
    position: [f32; 3],
    bounds: &PartBounds,
) -> ([u16; 4], [f32; 4]) {
    let Some(part) = part_of(legacy_id) else {
        return ([joints::HIPS as u16, 0, 0, 0], [1.0, 0.0, 0.0, 0.0]);
    };
    let side = BoneSide::of(position[0]);
    let t = bounds.fraction(part, position);

    let (primary, secondary, blend) = match part {
        BodyPart::Pelvis => {
            if t < 0.35 {
                (joints::HIPS, joints::PELVIS, (1.0 - smoothstep(0.0, 0.35, t)) * 0.6)
            } else {
                (joints::HIPS, joints::SPINE, smoothstep(0.75, 1.0, t) * 0.4)
            }
        }
        BodyPart::Spine | BodyPart::Torso => {
            // cintura → espinha → peito → peito superior
            if t < 0.30 {
                (joints::HIPS, joints::SPINE, smoothstep(0.05, 0.30, t))
            } else if t < 0.62 {
                (joints::SPINE, joints::CHEST, smoothstep(0.30, 0.62, t))
            } else {
                (joints::CHEST, joints::UPPER_CHEST, smoothstep(0.62, 0.92, t) * 0.85)
            }
        }
        BodyPart::Neck => {
            if t < 0.5 {
                (joints::NECK, joints::UPPER_CHEST, (1.0 - smoothstep(0.0, 0.5, t)) * 0.6)
            } else {
                (joints::NECK, joints::HEAD, smoothstep(0.5, 0.9, t))
            }
        }
        BodyPart::Head => (
            joints::HEAD,
            joints::NECK,
            (1.0 - smoothstep(0.0, 0.18, t)) * 0.5,
        ),
        BodyPart::Deltoid => (
            side.pick(joints::LEFT_SHOULDER, joints::RIGHT_SHOULDER),
            side.pick(joints::LEFT_CLAVICLE, joints::RIGHT_CLAVICLE),
            (1.0 - smoothstep(0.15, 0.65, t)) * 0.6,
        ),
        BodyPart::UpperArm => (
            side.pick(joints::LEFT_SHOULDER, joints::RIGHT_SHOULDER),
            side.pick(joints::LEFT_ELBOW, joints::RIGHT_ELBOW),
            smoothstep(0.55, 1.0, t) * 0.8,
        ),
        BodyPart::Forearm => (
            side.pick(joints::LEFT_ELBOW, joints::RIGHT_ELBOW),
            side.pick(joints::LEFT_WRIST, joints::RIGHT_WRIST),
            smoothstep(0.45, 1.0, t) * 0.8,
        ),
        BodyPart::Hand => (
            side.pick(joints::LEFT_WRIST, joints::RIGHT_WRIST),
            side.pick(joints::LEFT_ELBOW, joints::RIGHT_ELBOW),
            (1.0 - smoothstep(0.0, 0.25, t)) * 0.5,
        ),
        // t: 0 = joelho, 1 = quadril (o eixo Y cresce para cima)
        BodyPart::Thigh => {
            if t < 0.25 {
                (
                    side.pick(joints::LEFT_THIGH, joints::RIGHT_THIGH),
                    side.pick(joints::LEFT_KNEE, joints::RIGHT_KNEE),
                    (1.0 - smoothstep(0.0, 0.25, t)) * 0.55,
                )
            } else if t > 0.75 {
                (
                    side.pick(joints::LEFT_THIGH, joints::RIGHT_THIGH),
                    joints::HIPS,
                    smoothstep(0.75, 1.0, t) * 0.5,
                )
            } else {
                (side.pick(joints::LEFT_THIGH, joints::RIGHT_THIGH), 0, 0.0)
            }
        }
        // Bloco joelho+perna: t: 0 = tornozelo, 1 = joelho
        BodyPart::KneeCalf => {
            if t < 0.45 {
                (
                    side.pick(joints::LEFT_KNEE, joints::RIGHT_KNEE),
                    side.pick(joints::LEFT_ANKLE, joints::RIGHT_ANKLE),
                    (1.0 - smoothstep(0.0, 0.45, t)) * 0.6,
                )
            } else if t > 0.6 {
                (
                    side.pick(joints::LEFT_KNEE, joints::RIGHT_KNEE),
                    side.pick(joints::LEFT_THIGH, joints::RIGHT_THIGH),
                    smoothstep(0.6, 1.0, t) * 0.65,
                )
            } else {
                (side.pick(joints::LEFT_KNEE, joints::RIGHT_KNEE), 0, 0.0)
            }
        }
        BodyPart::Foot => (
            side.pick(joints::LEFT_ANKLE, joints::RIGHT_ANKLE),
            side.pick(joints::LEFT_TOES, joints::RIGHT_TOES),
            smoothstep(0.15, 0.85, t) * 0.7,
        ),
    };

    pack_influences(primary, secondary, blend)
}

fn pack_influences(primary: u32, secondary: u32, blend: f32) -> ([u16; 4], [f32; 4]) {
    debug_assert!((primary as usize) < CANONICAL_JOINT_COUNT);
    debug_assert!((secondary as usize) < CANONICAL_JOINT_COUNT);
    let blend = if blend.is_finite() { blend.clamp(0.0, 1.0) } else { 0.0 };
    if blend <= 1e-3 {
        return ([primary as u16, 0, 0, 0], [1.0, 0.0, 0.0, 0.0]);
    }
    if blend >= 0.999 {
        return ([secondary as u16, 0, 0, 0], [1.0, 0.0, 0.0, 0.0]);
    }
    (
        [primary as u16, secondary as u16, 0, 0],
        [1.0 - blend, blend, 0.0, 0.0],
    )
}

fn smoothstep(edge0: f32, edge1: f32, value: f32) -> f32 {
    if edge1 <= edge0 {
        return 0.0;
    }
    let t = ((value - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Paleta neutra (identidade) com `bone_count` ossos, no layout do shader.
///
/// Enquanto [`PROPORTIONS_BAKED_INTO_BASE_MESH`] for `true`, é esta a paleta
/// entregue: skinning e bake não podem aplicar a mesma deformação duas vezes.
pub fn identity_palette_floats(bone_count: usize) -> Vec<f32> {
    let mut palette = Vec::with_capacity(bone_count * 16);
    for _ in 0..bone_count {
        palette.extend_from_slice(&[
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        ]);
    }
    palette
}

/// `true` quando a paleta é a identidade (tolerância de 1e-5 por componente).
pub fn palette_is_identity(palette: &[[f32; 16]]) -> bool {
    const IDENTITY: [f32; 16] = [
        1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ];
    palette.iter().all(|matrix| {
        matrix
            .iter()
            .zip(IDENTITY.iter())
            .all(|(a, b)| (a - b).abs() <= 1e-5)
    })
}

/// `true` quando todos os floats de uma paleta achatada são a identidade.
pub fn flat_palette_is_identity(palette: &[f32]) -> bool {
    palette.len() % 16 == 0
        && palette
            .chunks_exact(16)
            .all(|matrix| {
                let mut array = [0.0f32; 16];
                array.copy_from_slice(matrix);
                palette_is_identity(&[array])
            })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deformation::canonical_base_mesh;
    use crate::mesh::BaseGender;

    fn skinned_canonical() -> (Mesh, SkinAssignmentSummary) {
        let mut mesh = canonical_base_mesh(BaseGender::Male);
        let summary = assign_legacy_skin_weights(&mut mesh);
        (mesh, summary)
    }

    fn legacy_ids_of(mesh: &Mesh) -> Vec<u16> {
        mesh.vertices
            .iter()
            .map(|vertex| vertex.joints[0])
            .collect()
    }

    #[test]
    fn every_vertex_gets_at_most_four_influences_that_sum_to_one() {
        let (mesh, summary) = skinned_canonical();
        assert!(summary.is_complete(), "atribuição incompleta: {summary:?}");
        assert!(summary.weights_normalized);
        assert!(summary.max_influences <= MAX_INFLUENCES);
        let blended = mesh
            .vertices
            .iter()
            .filter(|vertex| vertex.weights[1] > 0.0)
            .count();
        assert!(blended > 0, "a atribuição precisa ter transições entre ossos vizinhos");
        for vertex in &mesh.vertices {
            let sum: f32 = vertex.weights.iter().sum();
            assert!((sum - 1.0).abs() < 1e-5, "peso somando {sum}");
            for (slot, joint) in vertex.joints.iter().enumerate() {
                assert!(
                    (*joint as usize) < CANONICAL_JOINT_COUNT,
                    "osso {joint} fora do esqueleto de {CANONICAL_JOINT_COUNT}"
                );
                if vertex.weights[slot] > 0.0 {
                    assert!(*joint as usize > 0, "vértice influenciado pela raiz (osso 0)");
                }
            }
        }
    }

    #[test]
    fn generator_numbering_is_pinned_to_the_documented_legacy_ids() {
        // A atribuição **sobrescreve** os ids legados, então este teste pina a
        // numeração na fonte (a malha canônica é a única produtora deles). Se
        // `mesh.rs` renumerar uma parte, o teste cai aqui em vez de o personagem
        // sair deformado em silêncio.
        let source = include_str!("mesh.rs");
        let pairs: [(u16, u16, &str); 11] = [
            (4, 4, "Head"),
            (3, 3, "Neck"),
            (2, 2, "Torso"),
            (5, 9, "Deltoids"),
            (6, 10, "Upper Arms"),
            (7, 11, "Forearms"),
            (8, 12, "Hands"),
            (13, 16, "Thighs"),
            (14, 17, "Knees"),
            (15, 18, "Feet"),
            (0, 0, "Pelvis"),
        ];
        for (id, _, label) in pairs {
            assert!(
                source.contains(&format!("(Joint {id})")) || source.contains(&format!("(Joints {id}")),
                "a parte '{label}' precisa continuar marcada com o id legado {id}"
            );
        }
        for (left, right, label) in pairs {
            if left == right {
                continue;
            }
            assert!(
                source.contains(&format!(", {left}, {right},")),
                "o par '{label}' precisa continuar como ({left}, {right}) no gerador"
            );
        }
    }

    #[test]
    fn mapping_is_unique_per_part_and_never_maps_to_the_root_bone() {
        let bounds = PartBounds::neutral();
        let mut parts: Vec<BodyPart> = Vec::new();
        for id in 0..=18u16 {
            let part = part_of(id).unwrap_or_else(|| panic!("id legado {id} sem parte"));
            if !parts.contains(&part) {
                parts.push(part);
            }
            // osso da raiz (0) não recebe massa: ele é o pivô do mundo
            for x in [-0.4f32, 0.0, 0.4] {
                let (joints, weights) = skin_for_vertex(id, [x, 1.0, 0.0], &bounds);
                assert!(weights.iter().sum::<f32>() > 0.99);
                for (slot, joint) in joints.iter().take(2).enumerate() {
                    if weights[slot] > 0.0 {
                        assert_ne!(*joint as u32, joints::ROOT);
                    }
                }
            }
        }
        assert_eq!(parts.len(), BODY_PARTS.len(), "toda parte precisa ter um id legado");
        assert_eq!(part_of(legacy::SPINE), Some(BodyPart::Spine));
    }

    #[test]
    fn every_body_part_maps_to_a_canonical_bone_pair() {
        let mut ids: Vec<u16> = (0..=18).collect();
        ids.push(19);
        ids.push(255);
        let bounds = PartBounds::neutral();
        for id in ids {
            let (joints, weights) = skin_for_vertex(id, [0.3, 1.0, 0.1], &bounds);
            assert!((joints[0] as usize) < CANONICAL_JOINT_COUNT);
            assert!((joints[1] as usize) < CANONICAL_JOINT_COUNT);
            let sum: f32 = weights.iter().sum();
            assert!((sum - 1.0).abs() < 1e-5, "id {id} somando {sum}");
        }
        // id desconhecido vai para o quadril — nunca para a raiz sem massa
        let (unknown, _) = skin_for_vertex(255, [0.0, 1.0, 0.0], &bounds);
        assert_eq!(unknown[0], joints::HIPS as u16);
        assert!(part_of(255).is_none());
    }

    #[test]
    fn left_side_is_positive_x_and_right_side_is_negative_x() {
        // O lado vem da **geometria** (esqueleto canônico: esquerda em +X), não
        // dos rótulos do gerador, que são espelhados. Se alguém trocar isso, as
        // médias abaixo invertem de sinal.
        let (mesh, summary) = skinned_canonical();
        assert!(summary.left_is_positive_x);
        let left_chain = [
            joints::LEFT_CLAVICLE,
            joints::LEFT_SHOULDER,
            joints::LEFT_ELBOW,
            joints::LEFT_WRIST,
            joints::LEFT_THIGH,
            joints::LEFT_KNEE,
            joints::LEFT_ANKLE,
            joints::LEFT_TOES,
        ];
        let right_chain = [
            joints::RIGHT_CLAVICLE,
            joints::RIGHT_SHOULDER,
            joints::RIGHT_ELBOW,
            joints::RIGHT_WRIST,
            joints::RIGHT_THIGH,
            joints::RIGHT_KNEE,
            joints::RIGHT_ANKLE,
            joints::RIGHT_TOES,
        ];
        let (mut left_count, mut right_count) = (0usize, 0usize);
        let (mut left_sum, mut right_sum) = (0.0f32, 0.0f32);
        for vertex in &mesh.vertices {
            let primary = vertex.joints[0] as u32;
            if left_chain.contains(&primary) {
                left_count += 1;
                left_sum += vertex.position[0];
            }
            if right_chain.contains(&primary) {
                right_count += 1;
                right_sum += vertex.position[0];
            }
        }
        assert!(left_count > 100 && right_count > 100, "{left_count}/{right_count}");
        assert!(
            left_sum / (left_count as f32) > 0.0,
            "cadeia LEFT_* precisa ficar em +X (média {})",
            left_sum / left_count as f32
        );
        assert!(
            right_sum / (right_count as f32) < 0.0,
            "cadeia RIGHT_* precisa ficar em −X (média {})",
            right_sum / right_count as f32
        );
    }

    #[test]
    fn assigned_bones_land_on_the_right_height_of_the_body() {
        let (mesh, _) = skinned_canonical();
        for vertex in &mesh.vertices {
            let primary = vertex.joints[0] as u32;
            let y = vertex.position[1];
            if primary == joints::HEAD {
                assert!(y > 1.5, "osso da cabeça em y = {y}");
            }
            if matches!(primary, joints::LEFT_ANKLE | joints::RIGHT_ANKLE | joints::LEFT_TOES | joints::RIGHT_TOES)
            {
                assert!(y < 0.45, "osso do pé em y = {y}");
            }
            if matches!(primary, joints::LEFT_KNEE | joints::RIGHT_KNEE) {
                assert!(y < 0.75, "osso do joelho em y = {y}");
            }
        }
    }

    #[test]
    fn all_twelve_body_parts_are_covered_and_most_bones_are_used() {
        let (_, summary) = skinned_canonical();
        // 11 partes na malha: o id legado 1 (espinha) existe na tabela mas o
        // gerador não o emite — o tronco inteiro vem como TORSO.
        assert_eq!(summary.parts_used.len(), BODY_PARTS.len() - 1, "{:?}", summary.parts_used);
        assert!(!summary.parts_used.contains(&"spine"));
        for part in BODY_PARTS.iter().filter(|part| **part != BodyPart::Spine) {
            assert!(
                summary.parts_used.contains(&part.name()),
                "parte '{}' sem vértices",
                part.name()
            );
        }
        assert_eq!(summary.unmapped_vertices, 0);
        assert!(
            summary.joints_used.len() >= 20,
            "ossos usados: {:?}",
            summary.joints_used
        );
    }

    #[test]
    fn transitions_blend_towards_the_neighbouring_bone() {
        // Extremos de cada parte precisam misturar com o osso vizinho, senão o
        // membro rasga na junta.
        // Os limites saem de uma malha **fresca** (ids legados); medir depois da
        // atribuição leria ids canônicos como se fossem legados.
        let fresh = canonical_base_mesh(BaseGender::Male);
        let bounds = PartBounds::measure(&fresh);
        let extreme = |id: u16, axis: usize, highest: bool| -> [f32; 3] {
            fresh
                .vertices
                .iter()
                .filter(|vertex| vertex.joints[0] == id)
                .map(|vertex| vertex.position)
                .reduce(|a, b| {
                    let better = if highest { b[axis] > a[axis] } else { b[axis] < a[axis] };
                    if better {
                        b
                    } else {
                        a
                    }
                })
                .expect("a malha canônica precisa ter a parte")
        };

        // Os ids `*_RIGHT` do gerador ficam em x > 0 — que é o lado **esquerdo**
        // do esqueleto canônico (o espelho está documentado no topo do módulo).
        // topo do bloco joelho/perna (junto da coxa) mistura com a coxa
        let (joints_top, weights_top) = skin_for_vertex(
            legacy::KNEE_CALF_RIGHT,
            extreme(legacy::KNEE_CALF_RIGHT, 1, true),
            &bounds,
        );
        assert_eq!(joints_top[0], joints::LEFT_KNEE as u16);
        assert!(
            joints_top[1] == joints::LEFT_THIGH as u16 && weights_top[1] > 0.4,
            "topo do joelho precisa seguir a coxa: {joints_top:?} {weights_top:?}"
        );

        // base do bloco (junto do tornozelo) mistura com o tornozelo
        let (joints_bottom, weights_bottom) = skin_for_vertex(
            legacy::KNEE_CALF_RIGHT,
            extreme(legacy::KNEE_CALF_RIGHT, 1, false),
            &bounds,
        );
        assert_eq!(joints_bottom[0], joints::LEFT_KNEE as u16);
        assert!(
            joints_bottom[1] == joints::LEFT_ANKLE as u16 && weights_bottom[1] > 0.4,
            "base da perna precisa seguir o tornozelo: {joints_bottom:?} {weights_bottom:?}"
        );

        // frente do pé (junto dos dedos) mistura com os dedos
        let (joints_front, weights_front) =
            skin_for_vertex(legacy::FOOT_RIGHT, extreme(legacy::FOOT_RIGHT, 2, true), &bounds);
        assert_eq!(joints_front[0], joints::LEFT_ANKLE as u16);
        assert!(
            joints_front[1] == joints::LEFT_TOES as u16 && weights_front[1] > 0.3,
            "frente do pé precisa seguir os dedos: {joints_front:?} {weights_front:?}"
        );

        // e o espelho: o mesmo id do outro lado vira a cadeia RIGHT_*
        let (joints_mirror, _) =
            skin_for_vertex(legacy::FOOT_LEFT, extreme(legacy::FOOT_LEFT, 2, true), &bounds);
        assert_eq!(joints_mirror[0], joints::RIGHT_ANKLE as u16);
    }

    #[test]
    fn assignment_is_deterministic_and_idempotent() {
        let mut first_mesh = canonical_base_mesh(BaseGender::Male);
        let legacy = legacy_ids_of(&first_mesh);
        let first = assign_canonical_skin_weights(&mut first_mesh, &legacy);
        let before: Vec<([u16; 4], [f32; 4])> = first_mesh
            .vertices
            .iter()
            .map(|vertex| (vertex.joints, vertex.weights))
            .collect();

        // mesma entrada de novo → exatamente o mesmo resultado
        let second = assign_canonical_skin_weights(&mut first_mesh, &legacy);
        let after: Vec<([u16; 4], [f32; 4])> = first_mesh
            .vertices
            .iter()
            .map(|vertex| (vertex.joints, vertex.weights))
            .collect();
        assert_eq!(before, after, "atribuição precisa ser idempotente");
        assert_eq!(first, second);

        // e uma malha nova (outro processo de geração) dá o mesmo mapa
        let mut other = canonical_base_mesh(BaseGender::Male);
        assign_canonical_skin_weights(&mut other, &legacy);
        let elsewhere: Vec<([u16; 4], [f32; 4])> = other
            .vertices
            .iter()
            .map(|vertex| (vertex.joints, vertex.weights))
            .collect();
        assert_eq!(before, elsewhere, "a atribuição precisa ser determinística");
    }

    #[test]
    fn missing_legacy_ids_are_counted_instead_of_silently_hitting_bone_zero() {
        let mut mesh = canonical_base_mesh(BaseGender::Male);
        let short = vec![legacy::HEAD; 10];
        let summary = assign_canonical_skin_weights(&mut mesh, &short);
        assert_eq!(summary.unmapped_vertices, mesh.vertices.len() - 10);
        assert!(!summary.is_complete());
        for vertex in mesh.vertices.iter().take(10) {
            assert_ne!(u32::from(vertex.joints[0]), joints::ROOT);
        }
        for vertex in mesh.vertices.iter().skip(10) {
            assert_eq!(vertex.joints[0], joints::HIPS as u16);
            assert_eq!(vertex.weights[0], 1.0);
        }
    }

    #[test]
    fn identity_palette_keeps_skinning_neutral() {
        let (mesh, _) = skinned_canonical();
        let identity = identity_palette_floats(CANONICAL_JOINT_COUNT);
        assert_eq!(identity.len(), CANONICAL_JOINT_COUNT * 16);
        assert!(flat_palette_is_identity(&identity));

        let mut matrix = [0.0f32; 16];
        matrix.copy_from_slice(&identity[0..16]);
        for vertex in &mesh.vertices {
            let sum: f32 = vertex.weights.iter().sum();
            let position = vertex.position;
            let skinned = [
                matrix[0] * position[0] + matrix[4] * position[1] + matrix[8] * position[2] + matrix[12],
                matrix[1] * position[0] + matrix[5] * position[1] + matrix[9] * position[2] + matrix[13],
                matrix[2] * position[0] + matrix[6] * position[1] + matrix[10] * position[2] + matrix[14],
            ];
            for axis in 0..3 {
                assert!((skinned[axis] * sum - position[axis] * sum).abs() < 1e-4);
            }
        }
        assert!(palette_is_identity(&[matrix]));
        let mut scaled = matrix;
        scaled[0] = 2.0;
        assert!(!palette_is_identity(&[scaled]));
        assert!(!flat_palette_is_identity(&identity[..10]));
    }

    #[test]
    fn palette_covers_every_canonical_joint() {
        // A paleta entregue tem exatamente os 24 ossos do esqueleto canônico.
        let skeleton = crate::bone_sync::BondSyncManager::create_canonical_humanoid();
        assert_eq!(skeleton.joints.len(), CANONICAL_JOINT_COUNT);
        let bind_world = skeleton.compute_world_transforms();
        let matrices = skeleton.skinning_matrices(&bind_world);
        assert_eq!(matrices.len(), CANONICAL_JOINT_COUNT);
        let floats = crate::bone_sync::BondSyncManager::palette_floats(&matrices);
        assert_eq!(floats.len(), CANONICAL_JOINT_COUNT * 16);
        assert!(
            flat_palette_is_identity(&floats),
            "world × inverse bind da pose de repouso precisa ser a identidade"
        );
        assert_eq!(
            crate::bone_sync::BondSyncManager::palette_floats(&skeleton.identity_palette()).len(),
            CANONICAL_JOINT_COUNT * 16
        );
    }
}
