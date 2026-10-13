//! Controles macro: gênero/dimorfismo, pad de somatótipo, proporções e "resetar tudo".

use super::compose;
use anigo_core::command::{Command, CommandError};
use anigo_core::mesh::BaseGender;
use anigo_core::project::{CharacterProportions, CharacterState, ProjectState};
use anigo_core::somatotype::SomatotypeCoords;

// ---------------------------------------------------------------------------- gênero

/// Gênero base discreto a partir do dimorfismo contínuo (`>= 0.5` = masculino; mesma regra
/// do app web).
pub fn gender_for_dimorphism(g: f32) -> BaseGender {
    if g >= 0.5 {
        BaseGender::Male
    } else {
        BaseGender::Female
    }
}

/// Botões Masculino/Feminino: base canônica + dimorfismo nos extremos (1.0 / 0.0).
pub fn set_gender(state: &ProjectState, gender: BaseGender) -> Result<Option<Command>, CommandError> {
    let g = match gender {
        BaseGender::Male => 1.0,
        BaseGender::Female => 0.0,
    };
    compose(
        state,
        vec![Command::SetBaseGender { gender }, Command::SetGenderDimorphism { value: g }],
    )
}

/// Slider contínuo de dimorfismo: o comando do arrasto é só o dimorfismo; o gênero base
/// acompanha o lado do 0.5 ao soltar (`commit`).
pub fn dimorphism_preview(value: f32) -> Command {
    Command::SetGenderDimorphism { value: value.clamp(0.0, 1.0) }
}

/// Comando final do slider de dimorfismo: o dimorfismo e, junto, o gênero base do lado de
/// 0.5 em que parou. É montado SEM `compose`: no `commit_preview` o núcleo restaura o estado de
/// partida antes de aplicar, então comparar com o estado já prévisualizado descartaria tudo.
pub fn dimorphism_final(value: f32) -> Command {
    let v = value.clamp(0.0, 1.0);
    Command::Batch {
        commands: vec![
            Command::SetGenderDimorphism { value: v },
            Command::SetBaseGender { gender: gender_for_dimorphism(v) },
        ],
    }
}

// ------------------------------------------------------------------------ somatótipo

/// Geometria do pad (mesmo viewBox 200 x 180 do `SomatotypePad2D.svelte`).
pub const PAD_W: f32 = 200.0;
pub const PAD_H: f32 = 180.0;
pub const PAD_MESO: (f32, f32) = (100.0, 20.0);
pub const PAD_ENDO: (f32, f32) = (25.0, 160.0);
pub const PAD_ECTO: (f32, f32) = (175.0, 160.0);

/// Somatótipo (normalizado) → ponto no pad.
pub fn somatotype_to_pad(c: SomatotypeCoords) -> (f32, f32) {
    let n = c.normalized();
    (
        n.endomorph * PAD_ENDO.0 + n.mesomorph * PAD_MESO.0 + n.ectomorph * PAD_ECTO.0,
        n.endomorph * PAD_ENDO.1 + n.mesomorph * PAD_MESO.1 + n.ectomorph * PAD_ECTO.1,
    )
}

/// Ponto no pad → somatótipo (coordenadas baricêntricas; fora do triângulo é projetado ao
/// simplex e renormalizado).
pub fn pad_to_somatotype(px: f32, py: f32) -> SomatotypeCoords {
    let (mx, my) = PAD_MESO;
    let (ex, ey) = PAD_ENDO;
    let (cx, cy) = PAD_ECTO;
    let denom = (my - cy) * (ex - cx) + (cx - mx) * (ey - cy);
    if denom.abs() < 1e-6 || !px.is_finite() || !py.is_finite() {
        return SomatotypeCoords::default();
    }
    let l0 = ((my - cy) * (px - cx) + (cx - mx) * (py - cy)) / denom; // endo
    let l1 = ((cy - ey) * (px - cx) + (ex - cx) * (py - cy)) / denom; // meso
    let l2 = 1.0 - l0 - l1; // ecto
    SomatotypeCoords::new(l0.max(0.0), l1.max(0.0), l2.max(0.0)).normalized()
}

/// Comando do pad. Os componentes saem normalizados e dentro de `[0, 1]`.
pub fn somatotype_command(c: SomatotypeCoords) -> Command {
    let n = c.normalized();
    Command::SetSomatotype { endomorph: n.endomorph, mesomorph: n.mesomorph, ectomorph: n.ectomorph }
}

/// Atalhos do pad: (chave i18n, endo, meso, ecto) — mesmos valores do app web.
pub const SOMATOTYPE_SHORTCUTS: [(&str, f32, f32, f32); 4] = [
    ("somatotype.athletic", 0.10, 0.80, 0.10),
    ("somatotype.stocky", 0.75, 0.15, 0.10),
    ("somatotype.slender", 0.10, 0.15, 0.75),
    ("somatotype.balanced", 1.0 / 3.0, 1.0 / 3.0, 1.0 / 3.0),
];

// ------------------------------------------------------------------------ proporções

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Proportion {
    Height,
    HeadScale,
    HeadRatio,
    Shoulders,
    Legs,
    Arms,
    Neck,
    Torso,
}

impl Proportion {
    pub const ALL: [Proportion; 8] = [
        Proportion::Height,
        Proportion::HeadScale,
        Proportion::HeadRatio,
        Proportion::Shoulders,
        Proportion::Torso,
        Proportion::Legs,
        Proportion::Arms,
        Proportion::Neck,
    ];

    pub fn label_key(self) -> &'static str {
        match self {
            Proportion::Height => "prop.height",
            Proportion::HeadScale => "prop.head_scale",
            Proportion::HeadRatio => "prop.head_ratio",
            Proportion::Shoulders => "prop.shoulders",
            Proportion::Legs => "prop.legs",
            Proportion::Arms => "prop.arms",
            Proportion::Neck => "prop.neck",
            Proportion::Torso => "prop.torso",
        }
    }

    /// Faixa de edição (dentro do intervalo que `apply_proportions` aceita sem clamp).
    pub fn range(self) -> (f32, f32) {
        match self {
            Proportion::Height => (0.7, 1.4),
            Proportion::HeadScale => (0.6, 1.6),
            Proportion::HeadRatio => (2.5, 8.5),
            Proportion::Shoulders => (0.7, 1.5),
            Proportion::Legs => (0.7, 1.4),
            Proportion::Arms => (0.7, 1.4),
            Proportion::Neck => (0.6, 1.6),
            Proportion::Torso => (0.7, 1.4),
        }
    }

    pub fn default_value(self) -> f32 {
        self.get(&CharacterProportions::default())
    }

    pub fn get(self, p: &CharacterProportions) -> f32 {
        match self {
            Proportion::Height => p.height_overall,
            Proportion::HeadScale => p.head_scale,
            Proportion::HeadRatio => p.head_ratio,
            Proportion::Shoulders => p.shoulder_width,
            Proportion::Legs => p.leg_length,
            Proportion::Arms => p.arm_length,
            Proportion::Neck => p.neck_length,
            Proportion::Torso => p.torso_length,
        }
    }

    /// `SetProportions` com um único campo (clampado na faixa de edição).
    pub fn command(self, value: f32) -> Command {
        let (lo, hi) = self.range();
        let v = if value.is_finite() { value.clamp(lo, hi) } else { self.default_value() };
        let mut c = Command::SetProportions {
            head_scale: None,
            head_ratio: None,
            shoulder_width: None,
            leg_length: None,
            arm_length: None,
            neck_length: None,
            torso_length: None,
            height_overall: None,
        };
        if let Command::SetProportions {
            head_scale,
            head_ratio,
            shoulder_width,
            leg_length,
            arm_length,
            neck_length,
            torso_length,
            height_overall,
        } = &mut c
        {
            match self {
                Proportion::Height => *height_overall = Some(v),
                Proportion::HeadScale => *head_scale = Some(v),
                Proportion::HeadRatio => *head_ratio = Some(v),
                Proportion::Shoulders => *shoulder_width = Some(v),
                Proportion::Legs => *leg_length = Some(v),
                Proportion::Arms => *arm_length = Some(v),
                Proportion::Neck => *neck_length = Some(v),
                Proportion::Torso => *torso_length = Some(v),
            }
        }
        c
    }
}

/// `SetProportions` completo a partir de uma struct (usado por presets e reset).
pub fn proportions_command(p: &CharacterProportions) -> Command {
    Command::SetProportions {
        head_scale: Some(p.head_scale),
        head_ratio: Some(p.head_ratio),
        shoulder_width: Some(p.shoulder_width),
        leg_length: Some(p.leg_length),
        arm_length: Some(p.arm_length),
        neck_length: Some(p.neck_length),
        torso_length: Some(p.torso_length),
        height_overall: Some(p.height_overall),
    }
}

// ----------------------------------------------------------------------- resetar tudo

/// "Resetar tudo": morphs, gênero, dimorfismo, somatótipo e proporções voltam ao padrão do
/// projeto novo — um único passo de desfazer. `None` quando já está tudo no padrão.
pub fn reset_all(state: &ProjectState) -> Result<Option<Command>, CommandError> {
    let d = CharacterState::default();
    compose(
        state,
        vec![
            Command::ResetMorphs,
            Command::SetBaseGender { gender: d.base_gender },
            Command::SetGenderDimorphism { value: d.gender_dimorphism },
            somatotype_command(d.somatotype),
            proportions_command(&d.proportions),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use anigo_core::ids::MorphId;

    fn apply(state: &mut ProjectState, cmd: &Command) {
        cmd.apply(state).unwrap();
    }

    #[test]
    fn gender_buttons_set_base_and_extreme_dimorphism() {
        let mut s = ProjectState::default();
        let f = set_gender(&s, BaseGender::Female).unwrap().unwrap();
        assert!(f.validate(&s).is_ok());
        apply(&mut s, &f);
        assert_eq!(s.character.base_gender, BaseGender::Female);
        assert_eq!(s.character.gender_dimorphism, 0.0);
        // já feminino: nada a fazer
        assert!(set_gender(&s, BaseGender::Female).unwrap().is_none());
        let m = set_gender(&s, BaseGender::Male).unwrap().unwrap();
        apply(&mut s, &m);
        assert_eq!((s.character.base_gender, s.character.gender_dimorphism), (BaseGender::Male, 1.0));
    }

    #[test]
    fn dimorphism_final_follows_the_half_line() {
        let mut s = ProjectState::default();
        let c = dimorphism_final(0.2);
        assert!(c.validate(&s).is_ok());
        apply(&mut s, &c);
        assert_eq!(s.character.base_gender, BaseGender::Female);
        assert!((s.character.gender_dimorphism - 0.2).abs() < 1e-6);
        let c = dimorphism_final(0.5);
        apply(&mut s, &c);
        assert_eq!(s.character.base_gender, BaseGender::Male);
        assert!(matches!(dimorphism_preview(7.0), Command::SetGenderDimorphism { value } if value == 1.0));
    }

    #[test]
    fn pad_geometry_roundtrips_and_corners_are_pure() {
        for (e, m, c) in [(1.0, 0.0, 0.0), (0.0, 1.0, 0.0), (0.0, 0.0, 1.0), (0.2, 0.5, 0.3), (1.0 / 3.0, 1.0 / 3.0, 1.0 / 3.0)] {
            let p = SomatotypeCoords::new(e, m, c);
            let (x, y) = somatotype_to_pad(p);
            let back = pad_to_somatotype(x, y);
            assert!((back.endomorph - e).abs() < 1e-4 && (back.mesomorph - m).abs() < 1e-4 && (back.ectomorph - c).abs() < 1e-4, "{e} {m} {c} -> {back:?}");
        }
        assert_eq!(somatotype_to_pad(SomatotypeCoords::new(0.0, 1.0, 0.0)), PAD_MESO);
    }

    #[test]
    fn dragging_outside_the_triangle_projects_to_a_valid_somatotype() {
        for (x, y) in [(-50.0, -50.0), (500.0, 500.0), (100.0, -300.0), (f32::NAN, 3.0)] {
            let c = pad_to_somatotype(x, y);
            let cmd = somatotype_command(c);
            assert!(cmd.validate(&ProjectState::default()).is_ok(), "{x},{y}: {c:?}");
            let sum = c.endomorph + c.mesomorph + c.ectomorph;
            assert!((sum - 1.0).abs() < 1e-5);
        }
    }

    #[test]
    fn every_proportion_command_is_valid_and_touches_exactly_its_field() {
        let mut s = ProjectState::default();
        for p in Proportion::ALL {
            let (lo, hi) = p.range();
            for v in [lo, hi, f32::NAN, 99.0, -99.0] {
                let cmd = p.command(v);
                assert!(cmd.validate(&s).is_ok(), "{p:?} {v}");
            }
            let target = (lo + hi) * 0.5 + 0.03;
            let before = s.character.proportions;
            apply(&mut s, &p.command(target));
            assert!((p.get(&s.character.proportions) - target).abs() < 1e-6, "{p:?}");
            for other in Proportion::ALL.into_iter().filter(|o| *o != p) {
                assert_eq!(other.get(&s.character.proportions), other.get(&before), "{p:?} mexeu em {other:?}");
            }
        }
    }

    #[test]
    fn proportion_ranges_include_the_default_and_stay_inside_the_core_clamps() {
        for p in Proportion::ALL {
            let (lo, hi) = p.range();
            assert!(lo < p.default_value() && p.default_value() < hi, "{p:?}");
            assert!(lo >= 0.4 && hi <= 8.5);
        }
    }

    #[test]
    fn reset_all_restores_the_new_project_character() {
        let mut s = ProjectState::default();
        let fresh = s.clone();
        assert!(reset_all(&s).unwrap().is_none(), "projeto novo já está no padrão");
        apply(&mut s, &Command::SetMorphValue { target: MorphId::for_slider("head_width"), value: 1.2 });
        let g = set_gender(&s, BaseGender::Female).unwrap().unwrap();
        apply(&mut s, &g);
        apply(&mut s, &somatotype_command(SomatotypeCoords::new(0.7, 0.2, 0.1)));
        apply(&mut s, &Proportion::Height.command(1.2));
        let cmd = reset_all(&s).unwrap().unwrap();
        assert!(cmd.validate(&s).is_ok());
        apply(&mut s, &cmd);
        assert_eq!(s.character, fresh.character);
    }
}
