//! Presets canônicos de personagem (porte de `src/services/character_presets.ts`).
//!
//! Cada preset é DADO (tabela estática) e vira um `Command::Batch` do núcleo: um preset é
//! um único passo de desfazer, e o núcleo valida cada sub-comando.

use super::macros::{proportions_command, somatotype_command};
use super::compose;
use anigo_core::command::{Command, CommandError};
use anigo_core::ids::MorphId;
use anigo_core::mesh::BaseGender;
use anigo_core::project::{CharacterProportions, ProjectState};
use anigo_core::somatotype::SomatotypeCoords;

#[derive(Debug, Clone, Copy)]
pub struct CharacterPreset {
    /// Id estável (igual ao do app web).
    pub id: &'static str,
    pub male: bool,
    /// Heath-Carter (endo, meso, ecto).
    pub somatotype: (f32, f32, f32),
    pub gender_dimorphism: f32,
    pub head_scale: f32,
    pub head_ratio: f32,
    pub shoulder_width: f32,
    pub leg_length: f32,
    pub arm_length: f32,
    pub neck_length: f32,
    pub sliders: &'static [(&'static str, f32)],
}

impl CharacterPreset {
    pub fn name_key(&self) -> String {
        format!("preset.{}", self.id)
    }

    pub fn description_key(&self) -> String {
        format!("preset.{}.desc", self.id)
    }

    pub fn proportions(&self) -> CharacterProportions {
        CharacterProportions {
            head_scale: self.head_scale,
            head_ratio: self.head_ratio,
            shoulder_width: self.shoulder_width,
            leg_length: self.leg_length,
            arm_length: self.arm_length,
            neck_length: self.neck_length,
            // o app web não define tronco/altura nos presets: voltam ao padrão
            torso_length: 1.0,
            height_overall: 1.0,
        }
    }
}

pub const PRESETS: &[CharacterPreset] = &[
    CharacterPreset {
        id: "shonen_hero",
        male: true,
        somatotype: (0.15, 0.55, 0.30),
        gender_dimorphism: 1.0,
        head_scale: 1.00,
        head_ratio: 6.5,
        shoulder_width: 1.15,
        leg_length: 1.05,
        arm_length: 1.02,
        neck_length: 1.00,
        sliders: &[
            ("jaw_v_line_taper", 0.70),
            ("brow_ridge_prominence", 0.30),
            ("abs_sixpack_definition", 0.50),
            ("pectoral_muscle_bulk", 0.40),
            ("eye_canthal_tilt", 5.0),
            ("deltoid_muscle_volume", 0.45),
            ("latissimus_dorsi_flare", 0.40),
            ("thigh_circumference", 1.05),
        ],
    },
    CharacterPreset {
        id: "shojo_idol",
        male: false,
        somatotype: (0.30, 0.15, 0.55),
        gender_dimorphism: 0.0,
        head_scale: 1.05,
        head_ratio: 6.2,
        shoulder_width: 0.90,
        leg_length: 1.10,
        arm_length: 0.98,
        neck_length: 1.05,
        sliders: &[
            ("bust_volume_cup", 0.45),
            ("waist_pinch_width", 0.85),
            ("hip_trochanteric_flare", 1.20),
            ("eye_scale_uniform", 1.25),
            ("eye_canthal_tilt", -6.0),
            ("jaw_v_line_taper", 0.85),
            ("cheek_fullness_upper", 0.40),
            ("inner_thigh_gap", 0.30),
        ],
    },
    CharacterPreset {
        id: "muscular_berserker",
        male: true,
        somatotype: (0.10, 0.85, 0.05),
        gender_dimorphism: 1.0,
        head_scale: 0.90,
        head_ratio: 7.5,
        shoulder_width: 1.35,
        leg_length: 1.05,
        arm_length: 1.10,
        neck_length: 0.95,
        sliders: &[
            ("deltoid_muscle_volume", 0.90),
            ("biceps_peak_volume", 0.85),
            ("triceps_bulk", 0.80),
            ("pectoral_muscle_bulk", 0.90),
            ("trapezius_bulk", 0.85),
            ("abs_sixpack_definition", 0.90),
            ("jaw_bigonial_width", 1.25),
            ("neck_circumference", 1.35),
            ("thigh_circumference", 1.30),
            ("calf_circumference", 1.25),
        ],
    },
    CharacterPreset {
        id: "plus_size",
        male: false,
        somatotype: (0.75, 0.20, 0.05),
        gender_dimorphism: 0.0,
        head_scale: 1.02,
        head_ratio: 6.0,
        shoulder_width: 1.05,
        leg_length: 0.98,
        arm_length: 0.98,
        neck_length: 0.95,
        sliders: &[
            ("belly_visceral_protuberance", 0.65),
            ("flank_love_handles", 0.60),
            ("cheek_fullness_upper", 0.75),
            ("thigh_circumference", 1.35),
            ("hip_trochanteric_flare", 1.30),
            ("bust_volume_cup", 1.10),
            ("gluteus_volume_overall", 1.35),
            ("submental_fullness", 0.40),
        ],
    },
    CharacterPreset {
        id: "chibi",
        male: false,
        somatotype: (0.45, 0.10, 0.45),
        gender_dimorphism: 0.5,
        head_scale: 1.55,
        head_ratio: 2.5,
        shoulder_width: 0.75,
        leg_length: 0.75,
        arm_length: 0.75,
        neck_length: 0.75,
        sliders: &[
            ("eye_scale_uniform", 1.55),
            ("cheek_fullness_upper", 0.95),
            ("jaw_v_line_taper", 0.75),
            ("chin_length", 0.80),
            ("nose_tip_sharpness", 0.10),
            ("ear_scale_uniform", 1.20),
        ],
    },
    CharacterPreset {
        id: "heroic_8_5c",
        male: true,
        somatotype: (0.05, 0.60, 0.35),
        gender_dimorphism: 1.0,
        head_scale: 0.80,
        head_ratio: 8.5,
        shoulder_width: 1.25,
        leg_length: 1.35,
        arm_length: 1.20,
        neck_length: 1.15,
        sliders: &[
            ("jaw_v_line_taper", 0.85),
            ("deltoid_muscle_volume", 0.65),
            ("latissimus_dorsi_flare", 0.70),
            ("outer_thigh_sweep", 0.65),
            ("quadriceps_definition", 0.60),
            ("clavicle_bone_relief", 0.75),
            ("eye_canthal_tilt", 10.0),
        ],
    },
];

pub fn find(id: &str) -> Option<&'static CharacterPreset> {
    PRESETS.iter().find(|p| p.id == id)
}

/// Comandos do preset, em ordem: zera os morphs, define gênero/dimorfismo, somatótipo,
/// proporções e por fim cada slider.
pub fn preset_commands(p: &CharacterPreset) -> Vec<Command> {
    let mut cmds = vec![
        Command::ResetMorphs,
        Command::SetBaseGender {
            gender: if p.male { BaseGender::Male } else { BaseGender::Female },
        },
        Command::SetGenderDimorphism { value: p.gender_dimorphism },
        somatotype_command(SomatotypeCoords::new(p.somatotype.0, p.somatotype.1, p.somatotype.2)),
        proportions_command(&p.proportions()),
    ];
    cmds.extend(p.sliders.iter().map(|(id, v)| Command::SetMorphValue {
        target: MorphId::for_slider(id),
        value: *v,
    }));
    cmds
}

/// Comando de aplicar o preset ao estado atual: um `Batch` (um único passo de desfazer) sem
/// os passos que já estão no valor pedido. `Ok(None)` se o personagem já é exatamente o preset.
pub fn apply_preset(state: &ProjectState, p: &CharacterPreset) -> Result<Option<Command>, CommandError> {
    compose(state, preset_commands(p))
}

#[cfg(test)]
mod tests {
    use super::*;
    use anigo_core::command::CommandHistory;
    use anigo_core::morph_catalog::find_slider_def;
    use std::collections::HashSet;

    #[test]
    fn six_canonical_presets_with_unique_ids_and_translations() {
        assert_eq!(PRESETS.len(), 6);
        let ids: HashSet<_> = PRESETS.iter().map(|p| p.id).collect();
        assert_eq!(ids.len(), 6);
        for p in PRESETS {
            for k in [p.name_key(), p.description_key()] {
                assert_ne!(crate::i18n::tr_in(crate::i18n::Lang::PtBr, &k), k, "{k}");
                assert_ne!(crate::i18n::tr_in(crate::i18n::Lang::En, &k), k, "{k}");
            }
        }
    }

    #[test]
    fn preset_data_respects_the_catalog_and_the_canonical_polarity() {
        for p in PRESETS {
            let (e, m, c) = p.somatotype;
            assert!((e + m + c - 1.0).abs() < 1e-5, "{}: soma {}", p.id, e + m + c);
            assert!((0.0..=1.0).contains(&p.gender_dimorphism));
            assert!(if p.male { p.gender_dimorphism >= 0.5 } else { p.gender_dimorphism <= 0.5 }, "{}", p.id);
            for (id, v) in p.sliders {
                let d = find_slider_def(id).unwrap_or_else(|| panic!("{}: slider desconhecido {id}", p.id));
                assert!(*v >= d.min && *v <= d.max, "{}: {id}={v} fora de [{}, {}]", p.id, d.min, d.max);
            }
        }
    }

    #[test]
    fn every_preset_batch_validates_from_the_default_and_from_a_messy_state() {
        let fresh = ProjectState::default();
        let mut messy = ProjectState::default();
        for cmd in [
            Command::SetMorphValue { target: MorphId::for_slider("head_width"), value: 1.2 },
            Command::SetMorphValue { target: MorphId::for_slider("eye_scale_uniform"), value: 1.25 },
            Command::SetGenderDimorphism { value: 0.3 },
            Command::SetBaseGender { gender: BaseGender::Female },
        ] {
            cmd.apply(&mut messy).unwrap();
        }
        for state in [&fresh, &messy] {
            for p in PRESETS {
                let cmd = apply_preset(state, p).unwrap().unwrap_or_else(|| panic!("{} sem efeito", p.id));
                assert!(cmd.validate(state).is_ok(), "{}: {:?}", p.id, cmd.validate(state));
                assert!(matches!(cmd, Command::Batch { .. }));
            }
        }
    }

    #[test]
    fn applying_a_preset_replaces_the_look_and_is_one_undo_step() {
        let mut state = ProjectState::default();
        let original = state.clone();
        let mut history = CommandHistory::new(50);
        // estado sujo: um morph que o preset NÃO define precisa ser zerado
        history
            .execute(&mut state, Command::SetMorphValue { target: MorphId::for_slider("head_width"), value: 1.2 })
            .unwrap();
        let preset = find("chibi").unwrap();
        let cmd = apply_preset(&state, preset).unwrap().unwrap();
        let depth = history.undo_depth();
        history.execute(&mut state, cmd).unwrap();
        assert_eq!(history.undo_depth(), depth + 1, "preset = um passo");

        let c = &state.character;
        assert_eq!(c.proportions.head_ratio, 2.5);
        assert_eq!(c.gender_dimorphism, 0.5);
        assert_eq!(c.base_gender, BaseGender::Female);
        assert_eq!(state.morph_value("head_width"), 1.0, "morph fora do preset foi zerado");
        assert!((state.morph_value("eye_scale_uniform") - 1.55).abs() < 1e-6);
        assert!((c.somatotype.endomorph - 0.45).abs() < 1e-6);

        history.undo(&mut state).unwrap();
        assert!((state.morph_value("head_width") - 1.2).abs() < 1e-6);
        history.undo(&mut state).unwrap();
        assert_eq!(state, original);
    }

    #[test]
    fn applying_the_same_preset_twice_is_a_noop() {
        let mut state = ProjectState::default();
        for p in PRESETS {
            let cmd = apply_preset(&state, p).unwrap().unwrap();
            cmd.apply(&mut state).unwrap();
            assert!(apply_preset(&state, p).unwrap().is_none(), "{}", p.id);
        }
    }
}
