//! ANIGO Comprehensive Anatomical Morph Catalog.
//! Defines 148 anatomical and anime-stylized sliders across 18 zones,
//! provides canonical sparse morph delta generation on isomorphic base meshes,
//! and maintains bidirectional mapping for GPU compute accumulation.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::mesh::{BaseGender, Mesh};
use crate::morph::{MorphChannel, SparseMorphDelta, SparseMorphHeader, SparseMorphSet};

/// The 18 canonical anatomical regions and stylization zones.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AnatomicalZone {
    /// 1. Proporções Globais & Silhueta (8 sliders)
    GlobalSilhouette,
    /// 2. Crânio e Estrutura Craniofacial (11 sliders)
    Craniofacial,
    /// 3. Olhos e Órbitas Anime (12 sliders)
    Eyes,
    /// 4. Sobrancelhas Estruturais (6 sliders)
    Eyebrows,
    /// 5. Nariz Estilizado Anime (8 sliders)
    Nose,
    /// 6. Boca e Lábios (8 sliders)
    MouthLips,
    /// 7. Mandíbula, Queixo e Linha V (10 sliders)
    JawChin,
    /// 8. Orelhas Anime/Élficas (5 sliders)
    Ears,
    /// 9. Pescoço e Trapézio (7 sliders)
    NeckTrapezius,
    /// 10. Ombros, Clavículas e Dorsal (8 sliders)
    ShouldersClavicles,
    /// 11. Tórax e Peitorais Masculinos (6 sliders)
    ChestPectorals,
    /// 12. Busto e Glândulas Mamárias Femininas (9 sliders)
    BustFemale,
    /// 13. Abdômen, Cintura e Flancos (11 sliders)
    AbdomenWaist,
    /// 14. Pelve, Bacia e Curva do Quadril (9 sliders)
    PelvisHips,
    /// 15. Glúteos e Nádegas (9 sliders)
    Gluteus,
    /// 16. Braços, Antebraços e Cotovelos (10 sliders)
    UpperLimbs,
    /// 17. Mãos e Dedos (8 sliders)
    HandsFingers,
    /// 18. Pernas, Joelhos, Panturrilhas e Pés (12 sliders)
    LowerLimbs,
}

impl AnatomicalZone {
    pub fn name(&self) -> &'static str {
        match self {
            Self::GlobalSilhouette => "Proporções Globais & Silhueta",
            Self::Craniofacial => "Crânio e Estrutura Craniofacial",
            Self::Eyes => "Olhos e Órbitas Anime",
            Self::Eyebrows => "Sobrancelhas Estruturais",
            Self::Nose => "Nariz Estilizado Anime",
            Self::MouthLips => "Boca e Lábios",
            Self::JawChin => "Mandíbula, Queixo e Linha V",
            Self::Ears => "Orelhas Anime/Élficas",
            Self::NeckTrapezius => "Pescoço e Trapézio",
            Self::ShouldersClavicles => "Ombros, Clavículas e Dorsal",
            Self::ChestPectorals => "Tórax e Peitorais Masculinos",
            Self::BustFemale => "Busto e Glândulas Mamárias Femininas",
            Self::AbdomenWaist => "Abdômen, Cintura e Flancos",
            Self::PelvisHips => "Pelve, Bacia e Curva do Quadril",
            Self::Gluteus => "Glúteos e Nádegas",
            Self::UpperLimbs => "Braços, Antebraços e Cotovelos",
            Self::HandsFingers => "Mãos e Dedos",
            Self::LowerLimbs => "Pernas, Joelhos, Panturrilhas e Pés",
        }
    }
}

/// Deformation engine mechanism governing the slider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SliderMechanism {
    /// Pure GPU vertex blendshape delta accumulation
    Morph,
    /// Skeletal Joint Translation Offsets (BOND)
    BoneDelta,
    /// Hybrid coupling (Morph sculpt + Bone offset)
    Dual,
}

/// Gender dimorphic restriction or expression.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum GenderDimorphism {
    Both,
    MaleOnly,
    FemaleOnly,
}

/// Metadata definition for a single canonical anatomical morph slider.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MorphSliderDef {
    pub id: &'static str,
    pub name: &'static str,
    pub zone: AnatomicalZone,
    pub mechanism: SliderMechanism,
    pub min: f32,
    pub default_value: f32,
    pub max: f32,
    pub dimorphism: GenderDimorphism,
}

/// Exhaustive canonical catalog of the 157 anatomical and anime stylization sliders across 18 zones.
pub const ALL_MORPH_SLIDERS: [MorphSliderDef; 157] = [
    // ─────────────────────────────────────────────────────────────
    // 1. Proporções Globais & Silhueta (8 Sliders)
    // ─────────────────────────────────────────────────────────────
    MorphSliderDef { id: "height_overall", name: "Altura Estelar Total", zone: AnatomicalZone::GlobalSilhouette, mechanism: SliderMechanism::BoneDelta, min: 1.10, default_value: 1.65, max: 2.15, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "head_to_body_ratio", name: "Régua de Cabeças Canônica", zone: AnatomicalZone::GlobalSilhouette, mechanism: SliderMechanism::BoneDelta, min: 2.0, default_value: 6.5, max: 8.5, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "head_scale_uniform", name: "Escala do Crânio", zone: AnatomicalZone::GlobalSilhouette, mechanism: SliderMechanism::BoneDelta, min: 0.60, default_value: 1.00, max: 1.60, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "torso_to_limb_ratio", name: "Proporção Tronco/Pernas", zone: AnatomicalZone::GlobalSilhouette, mechanism: SliderMechanism::BoneDelta, min: 0.70, default_value: 1.00, max: 1.40, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "somatotype_endomorph", name: "Gordura / Corpulência Macro", zone: AnatomicalZone::GlobalSilhouette, mechanism: SliderMechanism::Morph, min: 0.00, default_value: 0.00, max: 1.00, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "somatotype_mesomorph", name: "Musculatura / Atletismo Macro", zone: AnatomicalZone::GlobalSilhouette, mechanism: SliderMechanism::Morph, min: 0.00, default_value: 0.00, max: 1.00, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "somatotype_ectomorph", name: "Magreza / Estrutura Óssea Macro", zone: AnatomicalZone::GlobalSilhouette, mechanism: SliderMechanism::Morph, min: 0.00, default_value: 0.00, max: 1.00, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "spine_s_curvature", name: "Curvatura de Postura S-Line", zone: AnatomicalZone::GlobalSilhouette, mechanism: SliderMechanism::Dual, min: -1.00, default_value: 0.00, max: 1.00, dimorphism: GenderDimorphism::Both },

    // ─────────────────────────────────────────────────────────────
    // 2. Crânio e Estrutura Craniofacial (11 Sliders)
    // ─────────────────────────────────────────────────────────────
    MorphSliderDef { id: "head_width", name: "Largura Biparietal", zone: AnatomicalZone::Craniofacial, mechanism: SliderMechanism::Dual, min: 0.75, default_value: 1.00, max: 1.35, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "head_depth", name: "Profundidade Occipital", zone: AnatomicalZone::Craniofacial, mechanism: SliderMechanism::BoneDelta, min: 0.80, default_value: 1.00, max: 1.30, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "face_lower_length", name: "Altura do Terço Inferior", zone: AnatomicalZone::Craniofacial, mechanism: SliderMechanism::Morph, min: 0.70, default_value: 1.00, max: 1.30, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "forehead_height", name: "Altura da Testa", zone: AnatomicalZone::Craniofacial, mechanism: SliderMechanism::Morph, min: 0.75, default_value: 1.00, max: 1.40, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "forehead_roundness", name: "Curvatura Frontal", zone: AnatomicalZone::Craniofacial, mechanism: SliderMechanism::Morph, min: 0.00, default_value: 0.50, max: 1.00, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "brow_ridge_prominence", name: "Arco Supraciliar", zone: AnatomicalZone::Craniofacial, mechanism: SliderMechanism::Morph, min: 0.00, default_value: 0.20, max: 1.00, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "temple_width", name: "Largura das Têmporas", zone: AnatomicalZone::Craniofacial, mechanism: SliderMechanism::Morph, min: 0.80, default_value: 1.00, max: 1.25, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "cheekbone_prominence", name: "Projeção Zigomática", zone: AnatomicalZone::Craniofacial, mechanism: SliderMechanism::Morph, min: 0.00, default_value: 0.30, max: 1.00, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "cheek_fullness_upper", name: "Volume Malar Superior", zone: AnatomicalZone::Craniofacial, mechanism: SliderMechanism::Morph, min: -1.00, default_value: 0.00, max: 1.00, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "cheek_hollow_lower", name: "Concavidade Bucal Inferior", zone: AnatomicalZone::Craniofacial, mechanism: SliderMechanism::Morph, min: 0.00, default_value: 0.00, max: 1.00, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "head_neck_blend", name: "Transição Cabeça-Pescoço", zone: AnatomicalZone::Craniofacial, mechanism: SliderMechanism::Morph, min: 0.00, default_value: 0.50, max: 1.00, dimorphism: GenderDimorphism::Both },

    // ─────────────────────────────────────────────────────────────
    // 3. Olhos e Órbitas Anime (12 Sliders)
    // ─────────────────────────────────────────────────────────────
    MorphSliderDef { id: "eye_scale_uniform", name: "Escala Geral dos Olhos", zone: AnatomicalZone::Eyes, mechanism: SliderMechanism::Morph, min: 0.60, default_value: 1.00, max: 1.60, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "eye_horizontal_width", name: "Largura Palpebral", zone: AnatomicalZone::Eyes, mechanism: SliderMechanism::Morph, min: 0.70, default_value: 1.00, max: 1.40, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "eye_vertical_height", name: "Abertura Vertical", zone: AnatomicalZone::Eyes, mechanism: SliderMechanism::Morph, min: 0.60, default_value: 1.00, max: 1.50, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "eye_interpupillary_dist", name: "Espaçamento Intercantal", zone: AnatomicalZone::Eyes, mechanism: SliderMechanism::Morph, min: 0.75, default_value: 1.00, max: 1.30, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "eye_vertical_position", name: "Posição Vertical na Face", zone: AnatomicalZone::Eyes, mechanism: SliderMechanism::Morph, min: -1.00, default_value: 0.00, max: 1.00, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "eye_socket_depth", name: "Profundidade Orbital", zone: AnatomicalZone::Eyes, mechanism: SliderMechanism::Morph, min: -0.50, default_value: 0.00, max: 0.50, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "eye_canthal_tilt", name: "Inclinação Cantal (Tsurime/Tareme)", zone: AnatomicalZone::Eyes, mechanism: SliderMechanism::Morph, min: -20.0, default_value: 0.0, max: 20.0, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "iris_scale_ratio", name: "Proporção da Íris", zone: AnatomicalZone::Eyes, mechanism: SliderMechanism::Morph, min: 0.60, default_value: 1.00, max: 1.40, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "upper_eyelid_fold", name: "Dobra Palpebral Superior", zone: AnatomicalZone::Eyes, mechanism: SliderMechanism::Morph, min: 0.00, default_value: 0.50, max: 1.00, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "lower_eyelid_aegyosal", name: "Volume da Bolsa Aegyosal", zone: AnatomicalZone::Eyes, mechanism: SliderMechanism::Morph, min: 0.00, default_value: 0.20, max: 1.00, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "eyelid_corner_curve", name: "Curvatura dos Cantos", zone: AnatomicalZone::Eyes, mechanism: SliderMechanism::Morph, min: -1.00, default_value: 0.00, max: 1.00, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "pupil_vertical_squash", name: "Formato Oval da Pupila", zone: AnatomicalZone::Eyes, mechanism: SliderMechanism::Morph, min: 0.70, default_value: 1.00, max: 1.30, dimorphism: GenderDimorphism::Both },

    // ─────────────────────────────────────────────────────────────
    // 4. Sobrancelhas Estruturais (6 Sliders)
    // ─────────────────────────────────────────────────────────────
    MorphSliderDef { id: "eyebrow_thickness", name: "Espessura da Sobrancelha", zone: AnatomicalZone::Eyebrows, mechanism: SliderMechanism::Morph, min: 0.50, default_value: 1.00, max: 2.00, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "eyebrow_arch_height", name: "Altura do Arco Superior", zone: AnatomicalZone::Eyebrows, mechanism: SliderMechanism::Morph, min: -1.00, default_value: 0.00, max: 1.00, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "eyebrow_inner_slant", name: "Inclinação Medial", zone: AnatomicalZone::Eyebrows, mechanism: SliderMechanism::Morph, min: -15.0, default_value: 0.0, max: 15.0, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "eyebrow_spacing", name: "Distância Glabelar", zone: AnatomicalZone::Eyebrows, mechanism: SliderMechanism::Morph, min: 0.70, default_value: 1.00, max: 1.40, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "eyebrow_length", name: "Comprimento Horizontal", zone: AnatomicalZone::Eyebrows, mechanism: SliderMechanism::Morph, min: 0.70, default_value: 1.00, max: 1.30, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "eyebrow_depth", name: "Projeção em Relevo", zone: AnatomicalZone::Eyebrows, mechanism: SliderMechanism::Morph, min: 0.00, default_value: 0.20, max: 1.00, dimorphism: GenderDimorphism::Both },

    // ─────────────────────────────────────────────────────────────
    // 5. Nariz Estilizado Anime (8 Sliders)
    // ─────────────────────────────────────────────────────────────
    MorphSliderDef { id: "nose_bridge_height", name: "Altura da Raiz Nasal", zone: AnatomicalZone::Nose, mechanism: SliderMechanism::Morph, min: 0.60, default_value: 1.00, max: 1.40, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "nose_bridge_depth", name: "Projeção de Perfil do Dorso", zone: AnatomicalZone::Nose, mechanism: SliderMechanism::Morph, min: 0.50, default_value: 1.00, max: 1.60, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "nose_length_vertical", name: "Comprimento Longitudinal", zone: AnatomicalZone::Nose, mechanism: SliderMechanism::Morph, min: 0.70, default_value: 1.00, max: 1.30, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "nose_tip_upturn", name: "Ângulo da Ponta (Arrebitado)", zone: AnatomicalZone::Nose, mechanism: SliderMechanism::Morph, min: -25.0, default_value: 0.0, max: 25.0, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "nose_tip_sharpness", name: "Afunilamento da Ponta", zone: AnatomicalZone::Nose, mechanism: SliderMechanism::Morph, min: 0.00, default_value: 0.70, max: 1.00, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "nose_alar_width", name: "Largura da Base Alar", zone: AnatomicalZone::Nose, mechanism: SliderMechanism::Morph, min: 0.60, default_value: 1.00, max: 1.40, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "nose_nostril_visibility", name: "Definição das Narinas", zone: AnatomicalZone::Nose, mechanism: SliderMechanism::Morph, min: 0.00, default_value: 0.20, max: 1.00, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "nose_profile_flatness", name: "Achatamento Anime Muzzle", zone: AnatomicalZone::Nose, mechanism: SliderMechanism::Morph, min: 0.00, default_value: 0.60, max: 1.00, dimorphism: GenderDimorphism::Both },

    // ─────────────────────────────────────────────────────────────
    // 6. Boca e Lábios (8 Sliders)
    // ─────────────────────────────────────────────────────────────
    MorphSliderDef { id: "mouth_width", name: "Largura da Rima Bucal", zone: AnatomicalZone::MouthLips, mechanism: SliderMechanism::Morph, min: 0.60, default_value: 1.00, max: 1.40, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "mouth_vertical_pos", name: "Posição Vertical / Filtro", zone: AnatomicalZone::MouthLips, mechanism: SliderMechanism::Morph, min: -1.00, default_value: 0.00, max: 1.00, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "mouth_depth_protrusion", name: "Projeção Dentofacial", zone: AnatomicalZone::MouthLips, mechanism: SliderMechanism::Morph, min: -0.50, default_value: 0.00, max: 0.50, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "lip_upper_thickness", name: "Espessura do Lábio Superior", zone: AnatomicalZone::MouthLips, mechanism: SliderMechanism::Morph, min: 0.20, default_value: 1.00, max: 2.00, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "lip_lower_thickness", name: "Espessura do Lábio Inferior", zone: AnatomicalZone::MouthLips, mechanism: SliderMechanism::Morph, min: 0.20, default_value: 1.00, max: 2.00, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "lip_corner_tilt", name: "Curvatura de Comissura", zone: AnatomicalZone::MouthLips, mechanism: SliderMechanism::Morph, min: -15.0, default_value: 0.0, max: 15.0, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "lip_philtrum_depth", name: "Profundidade do Filtro", zone: AnatomicalZone::MouthLips, mechanism: SliderMechanism::Morph, min: 0.00, default_value: 0.30, max: 1.00, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "mouth_tuck_depth", name: "Reentrância das Comissuras", zone: AnatomicalZone::MouthLips, mechanism: SliderMechanism::Morph, min: 0.00, default_value: 0.50, max: 1.00, dimorphism: GenderDimorphism::Both },

    // ─────────────────────────────────────────────────────────────
    // 7. Mandíbula, Queixo e Perfil Anime (10 Sliders)
    // ─────────────────────────────────────────────────────────────
    MorphSliderDef { id: "jaw_bigonial_width", name: "Largura Bigoníaca da Mandíbula", zone: AnatomicalZone::JawChin, mechanism: SliderMechanism::Morph, min: 0.70, default_value: 1.00, max: 1.35, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "jaw_angle_vertical", name: "Altura do Ramo da Mandíbula", zone: AnatomicalZone::JawChin, mechanism: SliderMechanism::Morph, min: 0.75, default_value: 1.00, max: 1.25, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "jaw_v_line_taper", name: "Afunilamento V-Line Anime", zone: AnatomicalZone::JawChin, mechanism: SliderMechanism::Morph, min: 0.00, default_value: 0.60, max: 1.00, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "chin_length", name: "Altura Vertical do Mento", zone: AnatomicalZone::JawChin, mechanism: SliderMechanism::Morph, min: 0.70, default_value: 1.00, max: 1.30, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "chin_width", name: "Largura da Ponta do Queixo", zone: AnatomicalZone::JawChin, mechanism: SliderMechanism::Morph, min: 0.50, default_value: 1.00, max: 1.50, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "chin_forward_projection", name: "Projeção do Pogônio", zone: AnatomicalZone::JawChin, mechanism: SliderMechanism::Morph, min: -0.50, default_value: 0.00, max: 0.50, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "chin_cleft_dimple", name: "Covinha no Queixo (Cleft)", zone: AnatomicalZone::JawChin, mechanism: SliderMechanism::Morph, min: 0.00, default_value: 0.00, max: 1.00, dimorphism: GenderDimorphism::MaleOnly },
    MorphSliderDef { id: "submental_fullness", name: "Gordura Submentoniana (Papada)", zone: AnatomicalZone::JawChin, mechanism: SliderMechanism::Morph, min: 0.00, default_value: 0.00, max: 1.00, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "jawline_bone_definition", name: "Nitidez da Borda Mandibular", zone: AnatomicalZone::JawChin, mechanism: SliderMechanism::Morph, min: 0.00, default_value: 0.70, max: 1.00, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "anime_profile_slant", name: "Slant de Perfil Anime", zone: AnatomicalZone::JawChin, mechanism: SliderMechanism::Morph, min: 0.00, default_value: 0.50, max: 1.00, dimorphism: GenderDimorphism::Both },

    // ─────────────────────────────────────────────────────────────
    // 8. Orelhas e Traços Élficos/Anime (5 Sliders)
    // ─────────────────────────────────────────────────────────────
    MorphSliderDef { id: "ear_scale_uniform", name: "Escala Auricular Total", zone: AnatomicalZone::Ears, mechanism: SliderMechanism::Morph, min: 0.60, default_value: 1.00, max: 1.50, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "ear_flare_angle", name: "Ângulo de Abertura (Orelha de Abano)", zone: AnatomicalZone::Ears, mechanism: SliderMechanism::Morph, min: 0.0, default_value: 15.0, max: 40.0, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "ear_pointy_elf", name: "Ponta Élfica / Fantasia", zone: AnatomicalZone::Ears, mechanism: SliderMechanism::Morph, min: 0.00, default_value: 0.00, max: 1.00, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "ear_lobe_length", name: "Comprimento do Lóbulo", zone: AnatomicalZone::Ears, mechanism: SliderMechanism::Morph, min: 0.50, default_value: 1.00, max: 1.50, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "ear_vertical_position", name: "Posição Vertical na Cabeça", zone: AnatomicalZone::Ears, mechanism: SliderMechanism::Morph, min: -1.00, default_value: 0.00, max: 1.00, dimorphism: GenderDimorphism::Both },

    // ─────────────────────────────────────────────────────────────
    // 9. Pescoço e Trapézio (7 Sliders)
    // ─────────────────────────────────────────────────────────────
    MorphSliderDef { id: "neck_length", name: "Comprimento do Pescoço", zone: AnatomicalZone::NeckTrapezius, mechanism: SliderMechanism::BoneDelta, min: 0.70, default_value: 1.00, max: 1.35, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "neck_circumference", name: "Circunferência e Espessura", zone: AnatomicalZone::NeckTrapezius, mechanism: SliderMechanism::Dual, min: 0.70, default_value: 1.00, max: 1.50, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "trapezius_bulk", name: "Volume Superior do Trapézio", zone: AnatomicalZone::NeckTrapezius, mechanism: SliderMechanism::Morph, min: 0.00, default_value: 0.20, max: 1.00, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "adams_apple_prominence", name: "Pomo de Adão", zone: AnatomicalZone::NeckTrapezius, mechanism: SliderMechanism::Morph, min: 0.00, default_value: 0.00, max: 1.00, dimorphism: GenderDimorphism::MaleOnly },
    MorphSliderDef { id: "scull_scm_tendon_relief", name: "Músculo Esternocleidomastóideo", zone: AnatomicalZone::NeckTrapezius, mechanism: SliderMechanism::Morph, min: 0.00, default_value: 0.30, max: 1.00, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "throat_concavity", name: "Concavidade da Fossa Jugular", zone: AnatomicalZone::NeckTrapezius, mechanism: SliderMechanism::Morph, min: 0.00, default_value: 0.50, max: 1.00, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "neck_forward_posture", name: "Projeção Anterior do Pescoço", zone: AnatomicalZone::NeckTrapezius, mechanism: SliderMechanism::BoneDelta, min: -15.0, default_value: 0.0, max: 20.0, dimorphism: GenderDimorphism::Both },

    // ─────────────────────────────────────────────────────────────
    // 10. Ombros, Clavículas e Escápulas (8 Sliders)
    // ─────────────────────────────────────────────────────────────
    MorphSliderDef { id: "shoulder_biacromial_width", name: "Largura Biacromial dos Ombros", zone: AnatomicalZone::ShouldersClavicles, mechanism: SliderMechanism::BoneDelta, min: 0.80, default_value: 1.00, max: 1.30, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "shoulder_acromial_slope", name: "Inclinação Acromial", zone: AnatomicalZone::ShouldersClavicles, mechanism: SliderMechanism::BoneDelta, min: -10.0, default_value: 0.0, max: 15.0, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "deltoid_muscle_volume", name: "Volume do Deltoide", zone: AnatomicalZone::ShouldersClavicles, mechanism: SliderMechanism::Morph, min: 0.00, default_value: 0.20, max: 1.00, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "clavicle_bone_relief", name: "Nitidez Óssea da Clavícula", zone: AnatomicalZone::ShouldersClavicles, mechanism: SliderMechanism::Morph, min: 0.00, default_value: 0.50, max: 1.00, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "clavicle_v_angle", name: "Inclinação em V das Clavículas", zone: AnatomicalZone::ShouldersClavicles, mechanism: SliderMechanism::Morph, min: -10.0, default_value: 0.0, max: 15.0, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "scapula_wing_relief", name: "Proeminência das Escápulas", zone: AnatomicalZone::ShouldersClavicles, mechanism: SliderMechanism::Morph, min: 0.00, default_value: 0.30, max: 1.00, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "shoulder_depth_thickness", name: "Espessura Glenoumeral", zone: AnatomicalZone::ShouldersClavicles, mechanism: SliderMechanism::Morph, min: 0.80, default_value: 1.00, max: 1.30, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "latissimus_dorsi_flare", name: "Expansão do Grande Dorsal", zone: AnatomicalZone::ShouldersClavicles, mechanism: SliderMechanism::Morph, min: 0.00, default_value: 0.10, max: 1.00, dimorphism: GenderDimorphism::Both },

    // ─────────────────────────────────────────────────────────────
    // 11. Tórax e Peitorais Masculinos (6 Sliders)
    // ─────────────────────────────────────────────────────────────
    MorphSliderDef { id: "ribcage_width", name: "Largura Torácica", zone: AnatomicalZone::ChestPectorals, mechanism: SliderMechanism::Dual, min: 0.80, default_value: 1.00, max: 1.30, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "ribcage_depth", name: "Profundidade do Tórax", zone: AnatomicalZone::ChestPectorals, mechanism: SliderMechanism::Morph, min: 0.80, default_value: 1.00, max: 1.30, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "pectoral_muscle_bulk", name: "Volume do Peitoral Maior", zone: AnatomicalZone::ChestPectorals, mechanism: SliderMechanism::Morph, min: 0.00, default_value: 0.20, max: 1.00, dimorphism: GenderDimorphism::MaleOnly },
    MorphSliderDef { id: "pectoral_lower_cut", name: "Definição da Linha Inframamária", zone: AnatomicalZone::ChestPectorals, mechanism: SliderMechanism::Morph, min: 0.00, default_value: 0.40, max: 1.00, dimorphism: GenderDimorphism::MaleOnly },
    MorphSliderDef { id: "pectoral_sternal_cleave", name: "Separação Esternal", zone: AnatomicalZone::ChestPectorals, mechanism: SliderMechanism::Morph, min: 0.00, default_value: 0.30, max: 1.00, dimorphism: GenderDimorphism::MaleOnly },
    MorphSliderDef { id: "sternum_hollow_depth", name: "Concavidade Esternal", zone: AnatomicalZone::ChestPectorals, mechanism: SliderMechanism::Morph, min: 0.00, default_value: 0.20, max: 1.00, dimorphism: GenderDimorphism::Both },

    // ─────────────────────────────────────────────────────────────
    // 12. Busto e Glândulas Mamárias Femininas (9 Sliders)
    // ─────────────────────────────────────────────────────────────
    MorphSliderDef { id: "bust_volume_cup", name: "Volume do Busto (Copa A a G)", zone: AnatomicalZone::BustFemale, mechanism: SliderMechanism::Morph, min: 0.00, default_value: 0.35, max: 1.50, dimorphism: GenderDimorphism::FemaleOnly },
    MorphSliderDef { id: "bust_vertical_position", name: "Altura de Inserção Torácica", zone: AnatomicalZone::BustFemale, mechanism: SliderMechanism::Morph, min: -1.00, default_value: 0.00, max: 1.00, dimorphism: GenderDimorphism::FemaleOnly },
    MorphSliderDef { id: "bust_separation_cleavage", name: "Distância Intermamária (Decote)", zone: AnatomicalZone::BustFemale, mechanism: SliderMechanism::Morph, min: 0.70, default_value: 1.00, max: 1.50, dimorphism: GenderDimorphism::FemaleOnly },
    MorphSliderDef { id: "bust_gravity_sag", name: "Caimento em Gota / Gravidade", zone: AnatomicalZone::BustFemale, mechanism: SliderMechanism::Morph, min: 0.00, default_value: 0.30, max: 1.00, dimorphism: GenderDimorphism::FemaleOnly },
    MorphSliderDef { id: "bust_firmness_roundness", name: "Firmeza / Turgor Adiposo", zone: AnatomicalZone::BustFemale, mechanism: SliderMechanism::Morph, min: 0.00, default_value: 0.70, max: 1.00, dimorphism: GenderDimorphism::FemaleOnly },
    MorphSliderDef { id: "bust_outward_angle", name: "Ângulo de Divergência Lateral", zone: AnatomicalZone::BustFemale, mechanism: SliderMechanism::Morph, min: 0.0, default_value: 10.0, max: 25.0, dimorphism: GenderDimorphism::FemaleOnly },
    MorphSliderDef { id: "bust_areola_diameter", name: "Diâmetro da Aréola", zone: AnatomicalZone::BustFemale, mechanism: SliderMechanism::Morph, min: 0.50, default_value: 1.00, max: 2.00, dimorphism: GenderDimorphism::FemaleOnly },
    MorphSliderDef { id: "bust_nipple_projection", name: "Projeção da Papila Mamária", zone: AnatomicalZone::BustFemale, mechanism: SliderMechanism::Morph, min: 0.00, default_value: 0.20, max: 1.00, dimorphism: GenderDimorphism::FemaleOnly },
    MorphSliderDef { id: "bust_underbust_taper", name: "Afunilamento Submamário", zone: AnatomicalZone::BustFemale, mechanism: SliderMechanism::Morph, min: 0.75, default_value: 1.00, max: 1.25, dimorphism: GenderDimorphism::FemaleOnly },

    // ─────────────────────────────────────────────────────────────
    // 13. Abdômen, Cintura e Flancos (11 Sliders)
    // ─────────────────────────────────────────────────────────────
    MorphSliderDef { id: "waist_pinch_width", name: "Estreitamento da Cintura", zone: AnatomicalZone::AbdomenWaist, mechanism: SliderMechanism::Dual, min: 0.65, default_value: 1.00, max: 1.40, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "waist_pinch_height", name: "Altura do Pinch da Cintura", zone: AnatomicalZone::AbdomenWaist, mechanism: SliderMechanism::BoneDelta, min: 0.80, default_value: 1.00, max: 1.20, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "belly_visceral_protuberance", name: "Projeção Abdominal Anterior", zone: AnatomicalZone::AbdomenWaist, mechanism: SliderMechanism::Morph, min: -0.50, default_value: 0.00, max: 1.00, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "belly_lower_panniculus", name: "Avental Adiposo Hipogástrico", zone: AnatomicalZone::AbdomenWaist, mechanism: SliderMechanism::Morph, min: 0.00, default_value: 0.00, max: 1.00, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "abs_sixpack_definition", name: "Relevo do Reto Abdominal", zone: AnatomicalZone::AbdomenWaist, mechanism: SliderMechanism::Morph, min: 0.00, default_value: 0.00, max: 1.00, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "oblique_apollo_belt", name: "Crista Ilíaca / Cinturão de Apolo", zone: AnatomicalZone::AbdomenWaist, mechanism: SliderMechanism::Morph, min: 0.00, default_value: 0.10, max: 1.00, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "flank_love_handles", name: "Gordura nos Flancos Laterais", zone: AnatomicalZone::AbdomenWaist, mechanism: SliderMechanism::Morph, min: 0.00, default_value: 0.00, max: 1.00, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "ribcage_costal_margin", name: "Projeção das Costelas Inferiores", zone: AnatomicalZone::AbdomenWaist, mechanism: SliderMechanism::Morph, min: 0.00, default_value: 0.20, max: 1.00, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "navel_vertical_pos", name: "Posição Vertical do Umbigo", zone: AnatomicalZone::AbdomenWaist, mechanism: SliderMechanism::Morph, min: -1.00, default_value: 0.00, max: 1.00, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "navel_shape_slit", name: "Formato em Fenda Vertical", zone: AnatomicalZone::AbdomenWaist, mechanism: SliderMechanism::Morph, min: 0.00, default_value: 0.50, max: 1.00, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "stomach_vacuum_depth", name: "Sucção Abdominal (Vacuum)", zone: AnatomicalZone::AbdomenWaist, mechanism: SliderMechanism::Morph, min: 0.00, default_value: 0.00, max: 1.00, dimorphism: GenderDimorphism::Both },

    // ─────────────────────────────────────────────────────────────
    // 14. Pelve, Bacia e Curva do Quadril (9 Sliders)
    // ─────────────────────────────────────────────────────────────
    MorphSliderDef { id: "pelvis_bicristal_width", name: "Largura Bicristal da Bacia", zone: AnatomicalZone::PelvisHips, mechanism: SliderMechanism::BoneDelta, min: 0.75, default_value: 1.00, max: 1.35, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "hip_trochanteric_flare", name: "Curvatura Bitrocantérica (Quadris)", zone: AnatomicalZone::PelvisHips, mechanism: SliderMechanism::Morph, min: 0.75, default_value: 1.00, max: 1.45, dimorphism: GenderDimorphism::FemaleOnly },
    MorphSliderDef { id: "pelvic_tilt_angle", name: "Anteversão / Retroversão Pélvica", zone: AnatomicalZone::PelvisHips, mechanism: SliderMechanism::BoneDelta, min: -15.0, default_value: 0.0, max: 20.0, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "iliac_crest_prominence", name: "Nitidez da Crista Ilíaca", zone: AnatomicalZone::PelvisHips, mechanism: SliderMechanism::Morph, min: 0.00, default_value: 0.40, max: 1.00, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "trochanteric_fat_saddlebag", name: "Depósito de Culotes Laterais", zone: AnatomicalZone::PelvisHips, mechanism: SliderMechanism::Morph, min: 0.00, default_value: 0.00, max: 1.00, dimorphism: GenderDimorphism::FemaleOnly },
    MorphSliderDef { id: "hip_dip_fill", name: "Preenchimento do Hip Dip", zone: AnatomicalZone::PelvisHips, mechanism: SliderMechanism::Morph, min: -1.00, default_value: 0.00, max: 1.00, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "pubic_arch_width", name: "Largura do Arco Púbico", zone: AnatomicalZone::PelvisHips, mechanism: SliderMechanism::Morph, min: 0.70, default_value: 1.00, max: 1.40, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "pelvis_depth", name: "Profundidade da Bacia", zone: AnatomicalZone::PelvisHips, mechanism: SliderMechanism::Morph, min: 0.80, default_value: 1.00, max: 1.30, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "groin_crease_depth", name: "Sulco da Dobra Inguinal", zone: AnatomicalZone::PelvisHips, mechanism: SliderMechanism::Morph, min: 0.00, default_value: 0.40, max: 1.00, dimorphism: GenderDimorphism::Both },

    // ─────────────────────────────────────────────────────────────
    // 15. Glúteos e Nádegas (9 Sliders)
    // ─────────────────────────────────────────────────────────────
    MorphSliderDef { id: "gluteus_volume_overall", name: "Volume Total das Nádegas", zone: AnatomicalZone::Gluteus, mechanism: SliderMechanism::Morph, min: 0.60, default_value: 1.00, max: 1.60, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "gluteus_posterior_shelf", name: "Projeção Posterior de Perfil", zone: AnatomicalZone::Gluteus, mechanism: SliderMechanism::Morph, min: 0.60, default_value: 1.00, max: 1.60, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "gluteus_lift_height", name: "Elevação Glútea Anti-Gravidade", zone: AnatomicalZone::Gluteus, mechanism: SliderMechanism::Morph, min: -1.00, default_value: 0.00, max: 1.00, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "gluteal_crease_depth", name: "Profundidade da Prega Infraglútea", zone: AnatomicalZone::Gluteus, mechanism: SliderMechanism::Morph, min: 0.00, default_value: 0.50, max: 1.00, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "gluteus_shape_profile", name: "Perfil Geométrico Glúteo", zone: AnatomicalZone::Gluteus, mechanism: SliderMechanism::Morph, min: -1.00, default_value: 0.00, max: 1.00, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "gluteus_medius_fill", name: "Volume Súpero-Lateral", zone: AnatomicalZone::Gluteus, mechanism: SliderMechanism::Morph, min: 0.00, default_value: 0.30, max: 1.00, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "gluteus_firmness", name: "Firmeza Muscular vs Flacidez", zone: AnatomicalZone::Gluteus, mechanism: SliderMechanism::Morph, min: 0.00, default_value: 0.60, max: 1.00, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "intergluteal_cleft_depth", name: "Profundidade do Sulco Central", zone: AnatomicalZone::Gluteus, mechanism: SliderMechanism::Morph, min: 0.00, default_value: 0.50, max: 1.00, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "infragluteal_crease_length", name: "Comprimento da Dobra Inferior", zone: AnatomicalZone::Gluteus, mechanism: SliderMechanism::Morph, min: 0.00, default_value: 0.50, max: 1.00, dimorphism: GenderDimorphism::Both },

    // ─────────────────────────────────────────────────────────────
    // 16. Braços, Antebraços e Cotovelos (10 Sliders)
    // ─────────────────────────────────────────────────────────────
    MorphSliderDef { id: "arm_length_overall", name: "Comprimento Total do Braço", zone: AnatomicalZone::UpperLimbs, mechanism: SliderMechanism::BoneDelta, min: 0.75, default_value: 1.00, max: 1.30, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "upper_arm_length", name: "Comprimento do Úmero", zone: AnatomicalZone::UpperLimbs, mechanism: SliderMechanism::BoneDelta, min: 0.80, default_value: 1.00, max: 1.25, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "forearm_length", name: "Comprimento Rádio-Ulna", zone: AnatomicalZone::UpperLimbs, mechanism: SliderMechanism::BoneDelta, min: 0.80, default_value: 1.00, max: 1.25, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "upper_arm_thickness", name: "Espessura Geral do Braço", zone: AnatomicalZone::UpperLimbs, mechanism: SliderMechanism::Dual, min: 0.70, default_value: 1.00, max: 1.50, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "biceps_peak_volume", name: "Pico do Bíceps Braquial", zone: AnatomicalZone::UpperLimbs, mechanism: SliderMechanism::Morph, min: 0.00, default_value: 0.20, max: 1.00, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "triceps_bulk", name: "Massa do Tríceps Braquial", zone: AnatomicalZone::UpperLimbs, mechanism: SliderMechanism::Morph, min: 0.00, default_value: 0.20, max: 1.00, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "forearm_brachioradialis", name: "Massa do Braquiorradial", zone: AnatomicalZone::UpperLimbs, mechanism: SliderMechanism::Morph, min: 0.00, default_value: 0.20, max: 1.00, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "forearm_taper_ratio", name: "Razão de Afunilamento", zone: AnatomicalZone::UpperLimbs, mechanism: SliderMechanism::Morph, min: 0.60, default_value: 1.00, max: 1.30, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "elbow_olecranon_sharpness", name: "Nitidez do Cotovelo (Olécrano)", zone: AnatomicalZone::UpperLimbs, mechanism: SliderMechanism::Morph, min: 0.00, default_value: 0.40, max: 1.00, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "wrist_circumference", name: "Circunferência do Punho", zone: AnatomicalZone::UpperLimbs, mechanism: SliderMechanism::Dual, min: 0.70, default_value: 1.00, max: 1.35, dimorphism: GenderDimorphism::Both },

    // ─────────────────────────────────────────────────────────────
    // 17. Mãos e Dedos (8 Sliders)
    // ─────────────────────────────────────────────────────────────
    MorphSliderDef { id: "hand_scale_uniform", name: "Escala da Mão", zone: AnatomicalZone::HandsFingers, mechanism: SliderMechanism::BoneDelta, min: 0.70, default_value: 1.00, max: 1.35, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "palm_width", name: "Largura Metacarpal da Palma", zone: AnatomicalZone::HandsFingers, mechanism: SliderMechanism::Dual, min: 0.75, default_value: 1.00, max: 1.35, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "palm_length", name: "Comprimento da Palma", zone: AnatomicalZone::HandsFingers, mechanism: SliderMechanism::BoneDelta, min: 0.80, default_value: 1.00, max: 1.25, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "finger_length", name: "Comprimento dos Dedos", zone: AnatomicalZone::HandsFingers, mechanism: SliderMechanism::BoneDelta, min: 0.75, default_value: 1.00, max: 1.30, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "finger_thickness", name: "Espessura das Falanges", zone: AnatomicalZone::HandsFingers, mechanism: SliderMechanism::Morph, min: 0.70, default_value: 1.00, max: 1.40, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "knuckle_joint_definition", name: "Relevo dos Nós dos Dedos", zone: AnatomicalZone::HandsFingers, mechanism: SliderMechanism::Morph, min: 0.00, default_value: 0.20, max: 1.00, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "thumb_opposability_angle", name: "Ângulo de Abertura do Polegar", zone: AnatomicalZone::HandsFingers, mechanism: SliderMechanism::BoneDelta, min: 0.70, default_value: 1.00, max: 1.30, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "fingernail_style_anime", name: "Formato das Unhas Anime", zone: AnatomicalZone::HandsFingers, mechanism: SliderMechanism::Morph, min: 0.00, default_value: 0.50, max: 1.00, dimorphism: GenderDimorphism::Both },

    // ─────────────────────────────────────────────────────────────
    // 18. Pernas, Joelhos, Panturrilhas e Pés (12 Sliders)
    // ─────────────────────────────────────────────────────────────
    MorphSliderDef { id: "leg_length_overall", name: "Comprimento Total da Perna", zone: AnatomicalZone::LowerLimbs, mechanism: SliderMechanism::BoneDelta, min: 0.75, default_value: 1.00, max: 1.40, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "thigh_length", name: "Comprimento do Fêmur", zone: AnatomicalZone::LowerLimbs, mechanism: SliderMechanism::BoneDelta, min: 0.80, default_value: 1.00, max: 1.30, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "thigh_circumference", name: "Circunferência da Coxa", zone: AnatomicalZone::LowerLimbs, mechanism: SliderMechanism::Dual, min: 0.70, default_value: 1.00, max: 1.45, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "inner_thigh_gap", name: "Espaçamento Adutor Medial", zone: AnatomicalZone::LowerLimbs, mechanism: SliderMechanism::Morph, min: -1.00, default_value: 0.00, max: 1.00, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "outer_thigh_sweep", name: "Curvatura Lateral da Coxa", zone: AnatomicalZone::LowerLimbs, mechanism: SliderMechanism::Morph, min: 0.00, default_value: 0.40, max: 1.00, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "quadriceps_definition", name: "Relevo do Reto Femoral/Vasto", zone: AnatomicalZone::LowerLimbs, mechanism: SliderMechanism::Morph, min: 0.00, default_value: 0.20, max: 1.00, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "knee_patella_prominence", name: "Projeção Óssea da Patela", zone: AnatomicalZone::LowerLimbs, mechanism: SliderMechanism::Morph, min: 0.00, default_value: 0.50, max: 1.00, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "knee_valgus_uchimata", name: "Alinhamento Valgo (Uchimata Anime)", zone: AnatomicalZone::LowerLimbs, mechanism: SliderMechanism::BoneDelta, min: -10.0, default_value: 0.0, max: 15.0, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "calf_circumference", name: "Circunferência da Panturrilha", zone: AnatomicalZone::LowerLimbs, mechanism: SliderMechanism::Dual, min: 0.70, default_value: 1.00, max: 1.40, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "gastrocnemius_height", name: "Altura do Ventre Muscular", zone: AnatomicalZone::LowerLimbs, mechanism: SliderMechanism::Morph, min: -1.00, default_value: 0.00, max: 1.00, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "ankle_malleolus_thickness", name: "Espessura do Tornozelo", zone: AnatomicalZone::LowerLimbs, mechanism: SliderMechanism::Dual, min: 0.70, default_value: 1.00, max: 1.35, dimorphism: GenderDimorphism::Both },
    MorphSliderDef { id: "foot_scale_and_arch", name: "Tamanho do Pé e Arco Plantar", zone: AnatomicalZone::LowerLimbs, mechanism: SliderMechanism::Dual, min: 0.75, default_value: 1.00, max: 1.30, dimorphism: GenderDimorphism::Both },
];

/// Finds a morph slider definition by unique identifier.
pub fn find_slider_def(id: &str) -> Option<&'static MorphSliderDef> {
    ALL_MORPH_SLIDERS.iter().find(|s| s.id == id)
}

/// Retrieves all sliders belonging to a specific anatomical zone.
pub fn get_sliders_for_zone(zone: AnatomicalZone) -> Vec<&'static MorphSliderDef> {
    ALL_MORPH_SLIDERS.iter().filter(|s| s.zone == zone).collect()
}

/// Mesh integrity verification metrics report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeshIntegrityReport {
    pub is_valid: bool,
    pub vertex_count: usize,
    pub triangle_count: usize,
    pub degenerate_triangle_count: usize,
    pub inverted_normal_count: usize,
    pub nan_or_inf_count: usize,
    pub min_bounds: [f32; 3],
    pub max_bounds: [f32; 3],
}

/// Generates canonical sparse morph targets for all 148 sliders tailored to the base mesh topology.
pub fn build_canonical_sparse_morph_set(base_mesh: &Mesh) -> SparseMorphSet {
    let mut morph_set = SparseMorphSet::new();

    // Canonical vertex ranges in the 4,070-vertex base mesh:
    // 0..425: Head & Face (center ~ y=1.60)
    // 425..544: Neck & Trapezius (y ~ 1.35..1.50)
    // 544..1069: Torso / Chest / Bust / Waist (y ~ 0.90..1.40)
    // 1069..1444: Pelvis / Gluteus (y ~ 0.75..0.95)
    // 1444..1678: Deltoids / Shoulders
    // 1678..1912: Upper Arms
    // 1912..2146: Forearms
    // 2146..2536: Hands & Fingers
    // 2536..2926: Thighs (y ~ 0.40..0.80)
    // 2926..3160: Knees (y ~ 0.35..0.45)
    // 3160..3550: Calves (y ~ 0.08..0.38)
    // 3550..4070: Feet & Toes (y ~ 0.00..0.10)

    for slider in &ALL_MORPH_SLIDERS {
        let mut deltas = Vec::new();

        for (v_idx, vert) in base_mesh.vertices.iter().enumerate() {
            let [x, y, z] = vert.position;
            let [nx, ny, nz] = vert.normal;

            let delta = match slider.id {
                // 1. Global & Craniofacial
                "head_width" => {
                    if v_idx < 425 {
                        let factor = (x.abs() * 8.0).min(1.0);
                        Some(([x.signum() * 0.025 * factor, 0.0, 0.0], [nx * 0.2, 0.0, 0.0]))
                    } else {
                        None
                    }
                }
                "head_depth" => {
                    if v_idx < 425 && z < 0.0 {
                        Some(([0.0, 0.0, -0.035], [0.0, 0.0, -0.2]))
                    } else {
                        None
                    }
                }
                "face_lower_length" => {
                    if v_idx < 425 && y < 1.62 && z > 0.0 {
                        let factor = ((1.62 - y) * 8.0).clamp(0.0, 1.0);
                        Some(([0.0, -0.025 * factor, 0.0], [0.0, -0.1, 0.0]))
                    } else {
                        None
                    }
                }
                "forehead_height" => {
                    if v_idx < 425 && y > 1.64 {
                        let factor = ((y - 1.64) * 10.0).min(1.0);
                        Some(([0.0, 0.030 * factor, 0.0], [0.0, 0.2, 0.0]))
                    } else {
                        None
                    }
                }
                "forehead_roundness" => {
                    if v_idx < 425 && y > 1.62 && z > 0.0 {
                        Some(([0.0, 0.008, 0.020], [0.0, 0.1, 0.3]))
                    } else {
                        None
                    }
                }
                "brow_ridge_prominence" => {
                    if v_idx < 425 && y > 1.58 && y < 1.66 && z > 0.03 {
                        Some(([0.0, 0.005, 0.022], [0.0, 0.0, 0.4]))
                    } else {
                        None
                    }
                }
                "cheekbone_prominence" => {
                    if v_idx < 425 && y > 1.53 && y < 1.62 && x.abs() > 0.04 && z > 0.02 {
                        Some(([x.signum() * 0.015, 0.0, 0.015], [nx * 0.3, 0.0, 0.3]))
                    } else {
                        None
                    }
                }
                "cheek_fullness_upper" => {
                    if v_idx < 425 && y > 1.50 && y < 1.60 && x.abs() > 0.02 && z > 0.03 {
                        Some(([x.signum() * 0.018, 0.005, 0.020], [nx * 0.3, 0.1, 0.4]))
                    } else {
                        None
                    }
                }
                "jaw_v_line_taper" => {
                    if v_idx < 425 && y < 1.56 && z > -0.02 {
                        let taper = ((1.56 - y) * 12.0).clamp(0.0, 1.0);
                        Some(([-x * 0.25 * taper, 0.0, 0.005 * taper], [-nx * 0.2, 0.0, 0.1]))
                    } else {
                        None
                    }
                }
                "jaw_bigonial_width" => {
                    if v_idx < 425 && y < 1.56 && x.abs() > 0.04 {
                        Some(([x.signum() * 0.025, 0.0, 0.0], [nx * 0.3, 0.0, 0.0]))
                    } else {
                        None
                    }
                }
                "chin_length" => {
                    if v_idx < 425 && y < 1.48 {
                        Some(([0.0, -0.020, 0.0], [0.0, -0.3, 0.0]))
                    } else {
                        None
                    }
                }
                "chin_forward_projection" => {
                    if v_idx < 425 && y < 1.52 && z > 0.04 {
                        Some(([0.0, 0.0, 0.025], [0.0, 0.0, 0.4]))
                    } else {
                        None
                    }
                }
                "chin_cleft_dimple" => {
                    if v_idx < 425 && y < 1.51 && x.abs() < 0.012 && z > 0.05 {
                        Some(([0.0, 0.0, -0.012], [0.0, 0.0, -0.3]))
                    } else {
                        None
                    }
                }

                // 2. Eyes Anime
                "eye_scale_uniform" => {
                    if v_idx < 425 && y > 1.54 && y < 1.65 && x.abs() > 0.025 && x.abs() < 0.075 && z > 0.04 {
                        let center_y = 1.60;
                        let dy = y - center_y;
                        Some(([x.signum() * 0.012, dy * 0.25, 0.008], [nx * 0.2, ny * 0.2, 0.2]))
                    } else {
                        None
                    }
                }
                "eye_canthal_tilt" => {
                    if v_idx < 425 && y > 1.55 && y < 1.65 && x.abs() > 0.03 && z > 0.04 {
                        let lateral_factor = ((x.abs() - 0.03) * 25.0).clamp(0.0, 1.0);
                        let tilt_scale = 0.015 / 20.0;
                        Some(([0.0, tilt_scale * lateral_factor, 0.0], [0.0, 0.015 * lateral_factor, 0.0]))
                    } else {
                        None
                    }
                }
                "lower_eyelid_aegyosal" => {
                    if v_idx < 425 && y > 1.54 && y < 1.58 && x.abs() > 0.03 && x.abs() < 0.065 && z > 0.05 {
                        Some(([0.0, -0.002, 0.012], [0.0, -0.1, 0.4]))
                    } else {
                        None
                    }
                }

                // 3. Nose & Slant
                "nose_bridge_depth" => {
                    if v_idx < 425 && y > 1.54 && y < 1.63 && x.abs() < 0.025 && z > 0.05 {
                        Some(([0.0, 0.0, 0.025], [0.0, 0.0, 0.5]))
                    } else {
                        None
                    }
                }
                "nose_tip_upturn" => {
                    if v_idx < 425 && y > 1.52 && y < 1.57 && x.abs() < 0.018 && z > 0.06 {
                        let scale = 1.0 / 25.0;
                        Some(([0.0, 0.015 * scale, 0.005 * scale], [0.0, 0.4 * scale, 0.2 * scale]))
                    } else {
                        None
                    }
                }
                "nose_tip_sharpness" => {
                    if v_idx < 425 && y > 1.52 && y < 1.57 && x.abs() < 0.022 && z > 0.055 {
                        Some(([-x * 0.35, 0.0, 0.012], [-nx * 0.3, 0.0, 0.3]))
                    } else {
                        None
                    }
                }
                "anime_profile_slant" => {
                    if v_idx < 425 && z > 0.03 && y < 1.62 {
                        let slant_factor = ((1.62 - y) * 5.0).clamp(0.0, 1.0);
                        Some(([0.0, 0.0, -0.018 * slant_factor], [0.0, 0.0, -0.2 * slant_factor]))
                    } else {
                        None
                    }
                }

                // 4. Ears
                "ear_pointy_elf" => {
                    if v_idx < 425 && x.abs() > 0.075 && y > 1.58 && z < 0.03 {
                        Some(([x.signum() * 0.040, 0.055, -0.025], [nx * 0.4, 0.5, -0.2]))
                    } else {
                        None
                    }
                }
                "ear_flare_angle" => {
                    if v_idx < 425 && x.abs() > 0.070 && z < 0.02 {
                        Some(([x.signum() * 0.030, 0.0, 0.010], [nx * 0.4, 0.0, 0.1]))
                    } else {
                        None
                    }
                }

                // 5. Neck & Trapezius
                "adams_apple_prominence" => {
                    if (425..544).contains(&v_idx) && x.abs() < 0.018 && z > 0.025 && y > 1.38 && y < 1.48 {
                        Some(([0.0, 0.0, 0.022], [0.0, 0.0, 0.5]))
                    } else {
                        None
                    }
                }
                "trapezius_bulk" => {
                    if (425..544).contains(&v_idx) && y < 1.42 && x.abs() > 0.035 {
                        Some(([x.signum() * 0.020, 0.025, 0.0], [nx * 0.3, 0.4, 0.0]))
                    } else {
                        None
                    }
                }
                "neck_circumference" => {
                    if (425..544).contains(&v_idx) {
                        Some(([nx * 0.018, 0.0, nz * 0.018], [nx * 0.2, 0.0, nz * 0.2]))
                    } else {
                        None
                    }
                }

                // 6. Chest & Bust
                "bust_volume_cup" => {
                    if (544..1069).contains(&v_idx) && z > 0.0 && y > 1.10 && y < 1.35 && x.abs() > 0.02 && x.abs() < 0.14 {
                        let y_bell = (-((y - 1.22) / 0.08).powi(2)).exp();
                        let x_bell = (-(((x.abs() - 0.065)) / 0.045).powi(2)).exp();
                        let intensity = y_bell * x_bell;
                        Some(([x.signum() * 0.010 * intensity, -0.008 * intensity, 0.055 * intensity], [nx * 0.2, -0.1, 0.5 * intensity]))
                    } else {
                        None
                    }
                }
                "bust_gravity_sag" => {
                    if (544..1069).contains(&v_idx) && z > 0.02 && y > 1.08 && y < 1.30 && x.abs() < 0.14 {
                        let intensity = (-((y - 1.18) / 0.07).powi(2)).exp();
                        Some(([0.0, -0.035 * intensity, -0.012 * intensity], [0.0, -0.4 * intensity, -0.1]))
                    } else {
                        None
                    }
                }
                "bust_separation_cleavage" => {
                    if (544..1069).contains(&v_idx) && z > 0.02 && y > 1.15 && y < 1.32 && x.abs() < 0.14 {
                        Some(([x.signum() * 0.025, 0.0, 0.0], [nx * 0.3, 0.0, 0.0]))
                    } else {
                        None
                    }
                }
                "pectoral_muscle_bulk" => {
                    if (544..1069).contains(&v_idx) && z > 0.0 && y > 1.15 && y < 1.38 && x.abs() < 0.16 {
                        let intensity = (-((y - 1.27) / 0.08).powi(2)).exp();
                        Some(([0.0, 0.005 * intensity, 0.030 * intensity], [0.0, 0.1, 0.4 * intensity]))
                    } else {
                        None
                    }
                }
                "pectoral_lower_cut" => {
                    if (544..1069).contains(&v_idx) && z > 0.02 && y > 1.14 && y < 1.20 && x.abs() < 0.15 {
                        Some(([0.0, -0.015, 0.005], [0.0, -0.3, 0.2]))
                    } else {
                        None
                    }
                }
                "latissimus_dorsi_flare" => {
                    if (544..1069).contains(&v_idx) && y > 1.05 && y < 1.32 && x.abs() > 0.10 && z < 0.05 {
                        Some(([x.signum() * 0.035, 0.0, -0.010], [nx * 0.4, 0.0, -0.2]))
                    } else {
                        None
                    }
                }
                "ribcage_width" => {
                    if (544..1069).contains(&v_idx) && y > 1.10 {
                        Some(([x.signum() * 0.028, 0.0, 0.0], [nx * 0.3, 0.0, 0.0]))
                    } else {
                        None
                    }
                }

                // 7. Abdomen & Waist
                "waist_pinch_width" => {
                    if (544..1069).contains(&v_idx) && y > 0.95 && y < 1.12 {
                        let pinch_factor = (1.0 - ((y - 1.03) / 0.08).abs()).max(0.0);
                        Some(([-x.signum() * 0.032 * pinch_factor, 0.0, 0.0], [-nx * 0.3 * pinch_factor, 0.0, 0.0]))
                    } else {
                        None
                    }
                }
                "abs_sixpack_definition" => {
                    if (544..1069).contains(&v_idx) && z > 0.04 && y > 0.92 && y < 1.20 && x.abs() < 0.09 {
                        let wave = (y * 42.0).cos();
                        let lateral_attenuation = (1.0 - (x / 0.09).powi(2)).max(0.0);
                        let displacement = wave * 0.012 * lateral_attenuation;
                        Some(([0.0, 0.0, displacement], [0.0, 0.0, wave * 0.3]))
                    } else {
                        None
                    }
                }
                "belly_visceral_protuberance" => {
                    if (544..1069).contains(&v_idx) && z > 0.01 && y > 0.90 && y < 1.15 {
                        let bump = (-((y - 1.02) / 0.10).powi(2)).exp() * (1.0 - (x / 0.14).powi(2)).max(0.0);
                        Some(([0.0, -0.008 * bump, 0.045 * bump], [0.0, -0.1, 0.5 * bump]))
                    } else {
                        None
                    }
                }
                "flank_love_handles" => {
                    if (544..1069).contains(&v_idx) && y > 0.88 && y < 1.02 && x.abs() > 0.08 {
                        Some(([x.signum() * 0.028, 0.0, 0.0], [nx * 0.3, 0.0, 0.0]))
                    } else {
                        None
                    }
                }

                // 8. Pelvis, Hips & Gluteus
                "hip_trochanteric_flare" => {
                    if (1069..1444).contains(&v_idx) && x.abs() > 0.08 {
                        let factor = (-((y - 0.84) / 0.08).powi(2)).exp();
                        Some(([x.signum() * 0.038 * factor, 0.0, 0.0], [nx * 0.4 * factor, 0.0, 0.0]))
                    } else {
                        None
                    }
                }
                "gluteus_volume_overall" => {
                    if (1069..1444).contains(&v_idx) && z < 0.0 {
                        let factor = (-((y - 0.82) / 0.08).powi(2)).exp() * (-(x / 0.14).powi(2)).exp();
                        Some(([0.0, 0.0, -0.050 * factor], [0.0, 0.0, -0.5 * factor]))
                    } else {
                        None
                    }
                }
                "gluteus_posterior_shelf" => {
                    if (1069..1444).contains(&v_idx) && z < -0.02 && y > 0.82 {
                        Some(([0.0, 0.005, -0.035], [0.0, 0.1, -0.4]))
                    } else {
                        None
                    }
                }
                "gluteus_shape_profile" => {
                    if (1069..1444).contains(&v_idx) && z < 0.0 {
                        let shape_shift = (y - 0.82) * 0.35;
                        Some(([0.0, 0.0, shape_shift * 0.025], [0.0, 0.0, shape_shift * 0.2]))
                    } else {
                        None
                    }
                }

                // 9. Upper Limbs
                "deltoid_muscle_volume" => {
                    if (1444..1678).contains(&v_idx) {
                        Some(([nx * 0.022, ny * 0.015, nz * 0.022], [nx * 0.3, ny * 0.2, nz * 0.3]))
                    } else {
                        None
                    }
                }
                "biceps_peak_volume" => {
                    if (1678..1912).contains(&v_idx) && z > 0.0 {
                        Some(([0.0, 0.0, 0.028], [0.0, 0.0, 0.4]))
                    } else {
                        None
                    }
                }
                "upper_arm_thickness" => {
                    if (1678..1912).contains(&v_idx) {
                        Some(([nx * 0.020, 0.0, nz * 0.020], [nx * 0.2, 0.0, nz * 0.2]))
                    } else {
                        None
                    }
                }
                "forearm_brachioradialis" => {
                    if (1912..2146).contains(&v_idx) && x.abs() > 0.25 {
                        Some(([x.signum() * 0.022, 0.0, 0.010], [nx * 0.3, 0.0, 0.1]))
                    } else {
                        None
                    }
                }

                // 10. Lower Limbs
                "thigh_circumference" => {
                    if (2536..2926).contains(&v_idx) {
                        Some(([nx * 0.025, 0.0, nz * 0.025], [nx * 0.2, 0.0, nz * 0.2]))
                    } else {
                        None
                    }
                }
                "inner_thigh_gap" => {
                    if (2536..2926).contains(&v_idx) && (x > 0.0 && nx < 0.0 || x < 0.0 && nx > 0.0) {
                        Some(([x.signum() * 0.022, 0.0, 0.0], [nx * 0.2, 0.0, 0.0]))
                    } else {
                        None
                    }
                }
                "outer_thigh_sweep" => {
                    if (2536..2926).contains(&v_idx) && (x > 0.0 && nx > 0.0 || x < 0.0 && nx < 0.0) && y > 0.55 {
                        Some(([x.signum() * 0.028, 0.0, 0.0], [nx * 0.3, 0.0, 0.0]))
                    } else {
                        None
                    }
                }
                "knee_patella_prominence" => {
                    if (2926..3160).contains(&v_idx) && z > 0.0 {
                        Some(([0.0, 0.0, 0.022], [0.0, 0.0, 0.4]))
                    } else {
                        None
                    }
                }
                "calf_circumference" => {
                    if (3160..3550).contains(&v_idx) {
                        Some(([nx * 0.022, 0.0, nz * 0.022], [nx * 0.2, 0.0, nz * 0.2]))
                    } else {
                        None
                    }
                }

                // Default fallback for any remaining morph sliders:
                //
                // P0 (canonical authority): the deformation model is
                // geometry-only, so *every* slider — including the BoneDelta
                // ones — receives the zone fallback delta. Bone-driven sliders
                // keep their skeletal semantics in `bone_sync.rs`; until real
                // skinning lands (P1-04) their geometric counterpart lives here
                // so no canonical slider is inert in the viewport/export.
                _ => {
                    match slider.zone {
                            AnatomicalZone::Craniofacial | AnatomicalZone::Eyes | AnatomicalZone::Nose | AnatomicalZone::MouthLips | AnatomicalZone::JawChin | AnatomicalZone::Ears | AnatomicalZone::Eyebrows => {
                                if v_idx < 425 && z > 0.0 {
                                    Some(([nx * 0.008, ny * 0.008, nz * 0.008], [nx * 0.1, ny * 0.1, nz * 0.1]))
                                } else {
                                    None
                                }
                            }
                            AnatomicalZone::NeckTrapezius => {
                                if (425..544).contains(&v_idx) {
                                    Some(([nx * 0.010, 0.0, nz * 0.010], [nx * 0.1, 0.0, nz * 0.1]))
                                } else {
                                    None
                                }
                            }
                            AnatomicalZone::ChestPectorals | AnatomicalZone::BustFemale | AnatomicalZone::AbdomenWaist | AnatomicalZone::ShouldersClavicles => {
                                if (544..1069).contains(&v_idx) {
                                    Some(([nx * 0.012, 0.0, nz * 0.012], [nx * 0.15, 0.0, nz * 0.15]))
                                } else {
                                    None
                                }
                            }
                            AnatomicalZone::PelvisHips | AnatomicalZone::Gluteus => {
                                if (1069..1444).contains(&v_idx) {
                                    Some(([nx * 0.015, 0.0, nz * 0.015], [nx * 0.15, 0.0, nz * 0.15]))
                                } else {
                                    None
                                }
                            }
                            AnatomicalZone::UpperLimbs | AnatomicalZone::HandsFingers => {
                                if (1444..2536).contains(&v_idx) {
                                    Some(([nx * 0.010, 0.0, nz * 0.010], [nx * 0.1, 0.0, nz * 0.1]))
                                } else {
                                    None
                                }
                            }
                            AnatomicalZone::LowerLimbs => {
                                if (2536..4070).contains(&v_idx) {
                                    Some(([nx * 0.012, 0.0, nz * 0.012], [nx * 0.1, 0.0, nz * 0.1]))
                                } else {
                                    None
                                }
                            }
                            AnatomicalZone::GlobalSilhouette => {
                                Some(([nx * 0.015, ny * 0.015, nz * 0.015], [nx * 0.1, ny * 0.1, nz * 0.1]))
                            }
                    }
                }
            };

            if let Some((d_pos, d_norm)) = delta {
                deltas.push(SparseMorphDelta::new(v_idx as u32, d_pos, d_norm));
            }
        }

        morph_set.add_target(slider.id, deltas);
    }

    // P1-05: nenhuma meta pode deformar posição sem normal. O catálogo canônico
    // já traz deltas de normal; quando uma meta customizada não trouxer, o
    // núcleo recalcula a partir da topologia e passa a entregá-la em delta —
    // WGSL/WebGL2/headless continuam sem precisar da topologia.
    morph_set.complete_normal_deltas(&base_mesh.vertices, &base_mesh.indices);

    morph_set
}

/// Active Morph Catalog manager maintaining current slider values and GPU delta buffers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MorphCatalog {
    pub gender: BaseGender,
    pub morph_set: SparseMorphSet,
    pub slider_values: HashMap<String, f32>,
}

impl MorphCatalog {
    pub fn new(gender: BaseGender) -> Self {
        let base_mesh = Mesh::create_canonical_base(gender);
        let morph_set = build_canonical_sparse_morph_set(&base_mesh);
        let mut slider_values = HashMap::with_capacity(ALL_MORPH_SLIDERS.len());

        for slider in &ALL_MORPH_SLIDERS {
            slider_values.insert(slider.id.to_string(), slider.default_value);
        }

        Self {
            gender,
            morph_set,
            slider_values,
        }
    }

    pub fn set_gender(&mut self, gender: BaseGender) {
        self.gender = gender;
        let base_mesh = Mesh::create_canonical_base(gender);
        self.morph_set = build_canonical_sparse_morph_set(&base_mesh);
    }

    pub fn set_slider(&mut self, id: &str, val: f32) -> Result<f32, String> {
        let def = find_slider_def(id).ok_or_else(|| format!("Unknown morph slider: '{}'", id))?;
        let clamped = val.clamp(def.min, def.max);
        self.slider_values.insert(id.to_string(), clamped);
        Ok(clamped)
    }

    pub fn get_slider(&self, id: &str) -> f32 {
        self.slider_values.get(id).copied().unwrap_or(0.0)
    }

    pub fn reset_all(&mut self) {
        for slider in &ALL_MORPH_SLIDERS {
            self.slider_values.insert(slider.id.to_string(), slider.default_value);
        }
    }

    pub fn get_active_morphs(&self) -> Vec<(&str, f32)> {
        let mut active = Vec::new();
        for slider in &ALL_MORPH_SLIDERS {
            let current = self.get_slider(slider.id);
            if (current - slider.default_value).abs() > 1e-5 {
                active.push((slider.id, current));
            }
        }
        active
    }

    pub fn pack_active_for_gpu(&self, vertex_count: u32) -> (SparseMorphHeader, Vec<MorphChannel>, Vec<SparseMorphDelta>) {
        let weights: Vec<f32> = self.morph_set.targets.iter().map(|target| {
            let val = self.slider_values.get(&target.name).copied().unwrap_or(0.0);
            if let Some(def) = find_slider_def(&target.name) {
                val - def.default_value
            } else {
                val
            }
        }).collect();

        self.morph_set.pack_active_channels(&weights, vertex_count, 1e-6)
    }

    pub fn apply_to_mesh(&self, base_mesh: &Mesh, out_mesh: &mut Mesh) {
        let weights: Vec<f32> = self.morph_set.targets.iter().map(|target| {
            let val = self.slider_values.get(&target.name).copied().unwrap_or(0.0);
            if let Some(def) = find_slider_def(&target.name) {
                val - def.default_value
            } else {
                val
            }
        }).collect();

        out_mesh.vertices.resize(base_mesh.vertices.len(), base_mesh.vertices[0]);
        self.morph_set.apply_cpu(&weights, &base_mesh.vertices, &mut out_mesh.vertices);
        out_mesh.indices = base_mesh.indices.clone();
    }

    /// Inspects geometric integrity of a mesh (detects degenerate faces, inverted normals, NaN coordinates).
    pub fn inspect_integrity(mesh: &Mesh) -> MeshIntegrityReport {
        let mut degenerate_count = 0;
        let mut inverted_count = 0;
        let mut nan_or_inf_count = 0;

        let mut min_bounds = [f32::INFINITY; 3];
        let mut max_bounds = [f32::NEG_INFINITY; 3];

        for v in &mesh.vertices {
            for i in 0..3 {
                let p = v.position[i];
                let n = v.normal[i];
                if p.is_nan() || p.is_infinite() || n.is_nan() || n.is_infinite() {
                    nan_or_inf_count += 1;
                }
                min_bounds[i] = min_bounds[i].min(p);
                max_bounds[i] = max_bounds[i].max(p);
            }
        }

        let tri_count = mesh.indices.len() / 3;
        for chunk in mesh.indices.chunks_exact(3) {
            let i0 = chunk[0] as usize;
            let i1 = chunk[1] as usize;
            let i2 = chunk[2] as usize;

            if i0 >= mesh.vertices.len() || i1 >= mesh.vertices.len() || i2 >= mesh.vertices.len() || i0 == i1 || i1 == i2 || i0 == i2 {
                degenerate_count += 1;
                continue;
            }

            let p0 = glam::Vec3::from_array(mesh.vertices[i0].position);
            let p1 = glam::Vec3::from_array(mesh.vertices[i1].position);
            let p2 = glam::Vec3::from_array(mesh.vertices[i2].position);

            let face_normal = (p1 - p0).cross(p2 - p0);
            let area_sq = face_normal.length_squared();

            if area_sq > 1e-8 {
                let avg_vert_norm = glam::Vec3::from_array(mesh.vertices[i0].normal)
                    + glam::Vec3::from_array(mesh.vertices[i1].normal)
                    + glam::Vec3::from_array(mesh.vertices[i2].normal);

                if face_normal.dot(avg_vert_norm) < -1e-6 {
                    inverted_count += 1;
                }
            }
        }

        let is_valid = nan_or_inf_count == 0 && degenerate_count == 0 && inverted_count == 0;

        MeshIntegrityReport {
            is_valid,
            vertex_count: mesh.vertices.len(),
            triangle_count: tri_count,
            degenerate_triangle_count: degenerate_count,
            inverted_normal_count: inverted_count,
            nan_or_inf_count,
            min_bounds,
            max_bounds,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_catalog_slider_count() {
        assert_eq!(ALL_MORPH_SLIDERS.len(), 157, "The catalog must contain all 157 sliders");

        let mut ids = std::collections::HashSet::new();
        for slider in &ALL_MORPH_SLIDERS {
            assert!(ids.insert(slider.id), "Duplicate slider ID found: {}", slider.id);
            assert!(slider.min <= slider.default_value, "Min must be <= default for {}", slider.id);
            assert!(slider.default_value <= slider.max, "Default must be <= max for {}", slider.id);
        }
    }

    #[test]
    fn test_catalog_covers_all_18_zones() {
        let mut zones = std::collections::HashSet::new();
        for slider in &ALL_MORPH_SLIDERS {
            zones.insert(slider.zone);
        }
        assert_eq!(zones.len(), 18, "All 18 zones must have assigned sliders in the catalog");
    }

    #[test]
    fn test_build_canonical_sparse_morph_set_sorted_indices() {
        let base_male = Mesh::create_canonical_base(BaseGender::Male);
        let morph_set = build_canonical_sparse_morph_set(&base_male);

        assert_eq!(morph_set.len(), 157, "Morph set must have 157 targets");

        for target in &morph_set.targets {
            for i in 1..target.deltas.len() {
                assert!(
                    target.deltas[i].vertex_index >= target.deltas[i - 1].vertex_index,
                    "Target {} deltas must be strictly sorted by vertex_index for binary search",
                    target.name
                );
            }
            for d in &target.deltas {
                assert!(
                    (d.vertex_index as usize) < base_male.vertices.len(),
                    "Vertex index {} out of bounds for target {}",
                    d.vertex_index,
                    target.name
                );
            }
        }
    }

    #[test]
    fn test_morph_catalog_application_and_mesh_integrity() {
        let mut catalog = MorphCatalog::new(BaseGender::Female);
        let base_female = Mesh::create_canonical_base(BaseGender::Female);

        // Baseline integrity
        let initial_report = MorphCatalog::inspect_integrity(&base_female);
        println!("Initial report: {:?}", initial_report);
        assert!(initial_report.is_valid);
        assert_eq!(initial_report.degenerate_triangle_count, 0);
        assert_eq!(initial_report.inverted_normal_count, 0);
        assert_eq!(initial_report.nan_or_inf_count, 0);

        // Apply several morphs
        catalog.set_slider("bust_volume_cup", 1.2).unwrap();
        catalog.set_slider("waist_pinch_width", 0.75).unwrap();
        catalog.set_slider("hip_trochanteric_flare", 1.35).unwrap();
        catalog.set_slider("jaw_v_line_taper", 0.90).unwrap();
        catalog.set_slider("eye_canthal_tilt", 15.0).unwrap();

        let mut morphed_mesh = base_female.clone();
        catalog.apply_to_mesh(&base_female, &mut morphed_mesh);

        let morphed_report = MorphCatalog::inspect_integrity(&morphed_mesh);
        println!("Morphed report: {:?}", morphed_report);
        assert!(morphed_report.is_valid, "Morphed mesh must preserve geometric integrity");
        assert_eq!(morphed_report.degenerate_triangle_count, 0);
        assert_eq!(morphed_report.inverted_normal_count, 0);

        // Verify active morphs
        let active = catalog.get_active_morphs();
        assert_eq!(active.len(), 5);

        // Pack for GPU
        let (header, channels, deltas) = catalog.pack_active_for_gpu(base_female.vertices.len() as u32);
        assert_eq!(header.active_channel_count, 5);
        assert_eq!(channels.len(), 5);
        assert!(!deltas.is_empty());
    }
}
