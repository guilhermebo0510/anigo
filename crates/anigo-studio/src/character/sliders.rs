//! Catálogo de sliders por região (Corpo/Rosto) e tradução slider ↔ `Command`.

use crate::i18n::{tr, Lang};
use anigo_core::command::Command;
use anigo_core::ids::MorphId;
use anigo_core::morph_catalog::{get_sliders_for_zone, AnatomicalZone, GenderDimorphism, MorphSliderDef};
use anigo_core::project::ProjectState;

/// Região do corpo editada por uma ferramenta do shelf.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Region {
    Body,
    Face,
}

/// Zonas do corpo (na ordem de exibição, de cima para baixo).
pub const BODY_ZONES: [AnatomicalZone; 11] = [
    AnatomicalZone::GlobalSilhouette,
    AnatomicalZone::NeckTrapezius,
    AnatomicalZone::ShouldersClavicles,
    AnatomicalZone::ChestPectorals,
    AnatomicalZone::BustFemale,
    AnatomicalZone::AbdomenWaist,
    AnatomicalZone::PelvisHips,
    AnatomicalZone::Gluteus,
    AnatomicalZone::UpperLimbs,
    AnatomicalZone::HandsFingers,
    AnatomicalZone::LowerLimbs,
];

/// Zonas do rosto.
pub const FACE_ZONES: [AnatomicalZone; 7] = [
    AnatomicalZone::Craniofacial,
    AnatomicalZone::Eyes,
    AnatomicalZone::Eyebrows,
    AnatomicalZone::Nose,
    AnatomicalZone::MouthLips,
    AnatomicalZone::JawChin,
    AnatomicalZone::Ears,
];

impl Region {
    pub fn zones(self) -> &'static [AnatomicalZone] {
        match self {
            Region::Body => &BODY_ZONES,
            Region::Face => &FACE_ZONES,
        }
    }

    /// Região de uma ferramenta do shelf (`body`/`face`); as demais não têm sliders.
    pub fn from_tool(id: &str) -> Option<Region> {
        match id {
            "body" => Some(Region::Body),
            "face" => Some(Region::Face),
            _ => None,
        }
    }
}

/// Chave de i18n do nome da zona.
pub fn zone_key(zone: AnatomicalZone) -> &'static str {
    match zone {
        AnatomicalZone::GlobalSilhouette => "zone.global_silhouette",
        AnatomicalZone::Craniofacial => "zone.craniofacial",
        AnatomicalZone::Eyes => "zone.eyes",
        AnatomicalZone::Eyebrows => "zone.eyebrows",
        AnatomicalZone::Nose => "zone.nose",
        AnatomicalZone::MouthLips => "zone.mouth_lips",
        AnatomicalZone::JawChin => "zone.jaw_chin",
        AnatomicalZone::Ears => "zone.ears",
        AnatomicalZone::NeckTrapezius => "zone.neck_trapezius",
        AnatomicalZone::ShouldersClavicles => "zone.shoulders_clavicles",
        AnatomicalZone::ChestPectorals => "zone.chest_pectorals",
        AnatomicalZone::BustFemale => "zone.bust_female",
        AnatomicalZone::AbdomenWaist => "zone.abdomen_waist",
        AnatomicalZone::PelvisHips => "zone.pelvis_hips",
        AnatomicalZone::Gluteus => "zone.gluteus",
        AnatomicalZone::UpperLimbs => "zone.upper_limbs",
        AnatomicalZone::HandsFingers => "zone.hands_fingers",
        AnatomicalZone::LowerLimbs => "zone.lower_limbs",
    }
}

/// Nome traduzido da zona.
pub fn zone_label(zone: AnatomicalZone) -> &'static str {
    tr(zone_key(zone))
}

/// Nome do slider no idioma: o catálogo do núcleo é pt-BR; em inglês deriva-se do id estável
/// (`jaw_v_line_taper` → "Jaw V Line Taper").
pub fn slider_label(def: &MorphSliderDef, lang: Lang) -> String {
    match lang {
        Lang::PtBr => def.name.to_string(),
        Lang::En => def
            .id
            .split('_')
            .filter(|w| !w.is_empty())
            .map(|w| {
                let mut cs = w.chars();
                match cs.next() {
                    Some(f) => f.to_uppercase().collect::<String>() + cs.as_str(),
                    None => String::new(),
                }
            })
            .collect::<Vec<_>>()
            .join(" "),
    }
}

/// Os 3 sliders `somatotype_*` são dirigidos pelo pad de somatótipo (um override manual
/// venceria o pad no núcleo), então não aparecem na lista de zonas.
pub fn is_macro_driven(def: &MorphSliderDef) -> bool {
    def.id.starts_with("somatotype_")
}

/// Comando de um slider; o valor é limitado ao domínio do catálogo (a UI nunca manda valor
/// fora do intervalo, e o núcleo ainda valida).
pub fn slider_command(def: &MorphSliderDef, value: f32) -> Command {
    let value = if value.is_finite() { value.clamp(def.min, def.max) } else { def.default_value };
    Command::SetMorphValue { target: MorphId::for_slider(def.id), value }
}

/// Comando que devolve o slider ao padrão do catálogo.
pub fn slider_reset_command(def: &MorphSliderDef) -> Command {
    slider_command(def, def.default_value)
}

/// Valor atual do slider no projeto (padrão do catálogo se não houver override).
pub fn slider_value(state: &ProjectState, def: &MorphSliderDef) -> f32 {
    state.morph_value(def.id)
}

/// `true` quando o slider difere do padrão (habilita o botão de reset).
pub fn is_modified(state: &ProjectState, def: &MorphSliderDef) -> bool {
    (slider_value(state, def) - def.default_value).abs() > 1e-6
}

/// Slider específico de um gênero que contradiz o dimorfismo atual (`0` feminino … `1` masculino).
pub fn is_dimorphism_dimmed(def: &MorphSliderDef, gender_dimorphism: f32) -> bool {
    match def.dimorphism {
        GenderDimorphism::Both => false,
        GenderDimorphism::MaleOnly => gender_dimorphism < 0.5,
        GenderDimorphism::FemaleOnly => gender_dimorphism >= 0.5,
    }
}

/// Casas decimais sugeridas pela largura do intervalo.
pub fn decimals(def: &MorphSliderDef) -> usize {
    let span = def.max - def.min;
    if span >= 50.0 {
        1
    } else if span >= 8.0 {
        2
    } else {
        3
    }
}

/// Busca por nome (nos dois idiomas) ou id; vazio casa tudo.
pub fn matches_query(def: &MorphSliderDef, query: &str) -> bool {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return true;
    }
    def.id.to_lowercase().replace('_', " ").contains(&q)
        || def.id.to_lowercase().contains(&q)
        || slider_label(def, Lang::PtBr).to_lowercase().contains(&q)
        || slider_label(def, Lang::En).to_lowercase().contains(&q)
}

/// Zonas da região com seus sliders já filtrados pela busca (zonas vazias somem).
pub fn visible_groups(region: Region, query: &str) -> Vec<(AnatomicalZone, Vec<&'static MorphSliderDef>)> {
    region
        .zones()
        .iter()
        .filter_map(|&zone| {
            let sliders: Vec<_> = get_sliders_for_zone(zone)
                .into_iter()
                .filter(|d| !is_macro_driven(d) && matches_query(d, query))
                .collect();
            (!sliders.is_empty()).then_some((zone, sliders))
        })
        .collect()
}

/// Quantidade de sliders modificados numa zona (badge do cabeçalho recolhível).
pub fn modified_in_zone(state: &ProjectState, zone: AnatomicalZone) -> usize {
    get_sliders_for_zone(zone).into_iter().filter(|d| is_modified(state, d)).count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use anigo_core::command::CommandError;
    use anigo_core::morph_catalog::{find_slider_def, ALL_MORPH_SLIDERS};
    use std::collections::HashSet;

    fn def(id: &str) -> &'static MorphSliderDef {
        find_slider_def(id).unwrap()
    }

    #[test]
    fn body_and_face_zones_partition_all_18_zones() {
        let mut seen = HashSet::new();
        for z in BODY_ZONES.iter().chain(FACE_ZONES.iter()) {
            assert!(seen.insert(*z), "zona repetida {z:?}");
        }
        assert_eq!(seen.len(), 18);
        let covered: usize = seen.iter().map(|z| get_sliders_for_zone(*z).len()).sum();
        assert_eq!(covered, ALL_MORPH_SLIDERS.len(), "157 sliders cobertos");
    }

    #[test]
    fn every_zone_has_a_translation_in_both_languages() {
        for z in BODY_ZONES.iter().chain(FACE_ZONES.iter()) {
            let k = zone_key(*z);
            assert_ne!(crate::i18n::tr_in(Lang::PtBr, k), k, "{k}");
            assert_ne!(crate::i18n::tr_in(Lang::En, k), k, "{k}");
        }
    }

    #[test]
    fn slider_value_maps_to_a_valid_command_and_back() {
        let mut state = ProjectState::default();
        for d in ALL_MORPH_SLIDERS.iter().filter(|d| !is_macro_driven(d)) {
            // 1) um ponto interior do intervalo sempre é aceito pelo núcleo
            let target = d.min + (d.max - d.min) * 0.7;
            let cmd = slider_command(d, target);
            match cmd.apply(&mut state) {
                Ok(_) => assert!((slider_value(&state, d) - target).abs() < 1e-5, "{}", d.id),
                Err(CommandError::NoOp(_)) => assert!((target - d.default_value).abs() < 1e-5, "{}", d.id),
                Err(e) => panic!("{}: {e}", d.id),
            }
            // 2) reset volta ao padrão
            let _ = slider_reset_command(d).apply(&mut state);
            assert!(!is_modified(&state, d), "{}", d.id);
        }
        assert!(state.character.morph_values.is_empty(), "tudo no padrão = nenhum override");
    }

    #[test]
    fn out_of_range_and_non_finite_values_are_clamped_not_rejected() {
        let d = def("head_width");
        let state = ProjectState::default();
        for v in [99.0, -99.0, f32::NAN, f32::INFINITY] {
            let r = slider_command(d, v).validate(&state);
            assert!(r.is_ok() || matches!(r, Err(CommandError::NoOp(_))), "{v}: {r:?}");
        }
        match slider_command(d, 99.0) {
            Command::SetMorphValue { value, .. } => assert_eq!(value, d.max),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn english_label_comes_from_the_stable_id() {
        assert_eq!(slider_label(def("jaw_v_line_taper"), Lang::En), "Jaw V Line Taper");
        assert_eq!(slider_label(def("head_width"), Lang::PtBr), def("head_width").name);
    }

    #[test]
    fn search_matches_name_or_id_in_either_language_and_hides_empty_zones() {
        let all: usize = visible_groups(Region::Face, "").iter().map(|(_, s)| s.len()).sum();
        assert!(all > 40);
        let eyes = visible_groups(Region::Face, "canthal");
        assert!(!eyes.is_empty());
        assert!(eyes.iter().all(|(z, _)| *z == AnatomicalZone::Eyes), "{eyes:?}");
        assert!(visible_groups(Region::Face, "zzzz-nada").is_empty());
        let first = ALL_MORPH_SLIDERS.iter().find(|d| d.zone == AnatomicalZone::Nose).unwrap();
        let word = first.name.split_whitespace().next().unwrap().to_lowercase();
        assert!(visible_groups(Region::Face, &word).iter().any(|(z, _)| *z == AnatomicalZone::Nose));
    }

    #[test]
    fn somatotype_macro_sliders_are_not_listed() {
        let body: usize = visible_groups(Region::Body, "").iter().map(|(_, s)| s.len()).sum();
        let face: usize = visible_groups(Region::Face, "").iter().map(|(_, s)| s.len()).sum();
        assert_eq!(body + face + 3, 157);
        assert!(visible_groups(Region::Body, "somatotype").is_empty());
    }

    #[test]
    fn dimorphism_dimming_follows_gender() {
        let bust = ALL_MORPH_SLIDERS.iter().find(|d| d.dimorphism == GenderDimorphism::FemaleOnly).unwrap();
        assert!(is_dimorphism_dimmed(bust, 1.0));
        assert!(!is_dimorphism_dimmed(bust, 0.0));
        let male = ALL_MORPH_SLIDERS.iter().find(|d| d.dimorphism == GenderDimorphism::MaleOnly).unwrap();
        assert!(is_dimorphism_dimmed(male, 0.0));
        assert!(!is_dimorphism_dimmed(def("head_width"), 0.0));
    }

    #[test]
    fn region_from_tool_ids() {
        assert_eq!(Region::from_tool("body"), Some(Region::Body));
        assert_eq!(Region::from_tool("face"), Some(Region::Face));
        assert_eq!(Region::from_tool("hair"), None);
    }
}
