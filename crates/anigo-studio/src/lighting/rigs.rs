//! Rigs de luz (presets) do workspace Iluminação.
//!
//! O contrato de render tem UMA luz direcional principal (+ ambiente hemisférico + o rim do material),
//! então um "rig" aqui é uma combinação honesta desses três: **key** = a direcional, **fill** = o
//! ambiente hemisférico (no shader o ambiente MULTIPLICA o tom da sombra: ambiente ≈ 1,5 dá o contraste
//! ~3:1 do fill de um rig de três pontos, conferido por teste) e **rim** =
//! o rim Fresnel + contraluz do material do personagem. Aplicar um rig é UM `Command::Batch`
//! (um passo de desfazer).
//!
//! Fontes das receitas: regra de três pontos (key 15°–70°, mais comum 45°; fill 3:1; back light atrás do
//! sujeito) em https://2822digitalcinematography.wordpress.com/three-point-lighting-the-basic-setup/ e
//! StudioBinder; Guilty Gear Xrd usa um vetor de luz por personagem (Motomura, GDC 2015). Os números são
//! ponto de partida artístico, ajustáveis — não calibrados em GPU real.

use super::{dir_from_az_el, kelvin_to_rgb};
use anigo_core::command::{Command, MaterialPatch};
use anigo_core::ProjectState;

#[derive(Debug, Clone, Copy)]
pub struct RigSpec {
    pub id: &'static str,
    /// Chave de i18n do rótulo (`light.rig.<id>`) e da dica (`light.rig.<id>.tip`).
    pub label_key: &'static str,
    pub tip_key: &'static str,
    pub icon: &'static str,
    /// Azimute / elevação do sol em graus (mundo: 0° = frente do personagem).
    pub az: f32,
    pub el: f32,
    /// Temperatura da luz (K) → cor via [`kelvin_to_rgb`].
    pub kelvin: f32,
    pub intensity: f32,
    pub ambient: f32,
    pub sky: [f32; 3],
    pub ground: [f32; 3],
    /// Tinta e saturação da sombra (a cor de sombra do material × esta tinta).
    pub shadow_tint: [f32; 3],
    pub shadow_saturation: f32,
    /// Rim do material do personagem: intensidade, abertura e cor.
    pub rim_intensity: f32,
    pub rim_spread: f32,
    pub rim_color: [f32; 4],
}

macro_rules! rig {
    ($id:literal, $icon:expr, $($field:ident: $v:expr),* $(,)?) => {
        RigSpec {
            id: $id,
            label_key: concat!("light.rig.", $id),
            tip_key: concat!("light.rig.", $id, ".tip"),
            icon: $icon,
            $($field: $v),*
        }
    };
}

/// O catálogo, na ordem da UI.
pub const RIGS: [RigSpec; 7] = [
    rig!("three_point", crate::icons::LIGHTBULB,
        az: -40.0, el: 35.0, kelvin: 5600.0, intensity: 1.0, ambient: 1.5,
        sky: [0.62, 0.68, 0.85], ground: [0.38, 0.32, 0.30],
        shadow_tint: [1.0, 1.0, 1.0], shadow_saturation: 1.0,
        rim_intensity: 1.1, rim_spread: 0.40, rim_color: [0.58, 0.77, 0.99, 1.0]),
    rig!("rim", crate::icons::FLASHLIGHT,
        az: 165.0, el: 28.0, kelvin: 6500.0, intensity: 1.25, ambient: 1.9,
        sky: [0.45, 0.50, 0.70], ground: [0.20, 0.18, 0.20],
        shadow_tint: [1.0, 1.0, 1.0], shadow_saturation: 1.0,
        rim_intensity: 2.2, rim_spread: 0.55, rim_color: [1.0, 0.90, 0.75, 1.0]),
    rig!("day", crate::icons::SUN,
        az: 25.0, el: 62.0, kelvin: 6200.0, intensity: 1.15, ambient: 1.7,
        sky: [0.55, 0.72, 1.0], ground: [0.55, 0.50, 0.42],
        shadow_tint: [0.88, 0.92, 1.0], shadow_saturation: 1.05,
        rim_intensity: 0.6, rim_spread: 0.35, rim_color: [0.80, 0.90, 1.0, 1.0]),
    rig!("golden_hour", crate::icons::SUN_HORIZON,
        az: -72.0, el: 11.0, kelvin: 3200.0, intensity: 1.2, ambient: 1.6,
        sky: [0.98, 0.65, 0.50], ground: [0.30, 0.20, 0.20],
        shadow_tint: [0.78, 0.72, 1.0], shadow_saturation: 1.15,
        rim_intensity: 1.4, rim_spread: 0.45, rim_color: [1.0, 0.72, 0.45, 1.0]),
    rig!("night", crate::icons::MOON_STARS,
        az: 55.0, el: 42.0, kelvin: 9500.0, intensity: 0.5, ambient: 1.2,
        sky: [0.14, 0.20, 0.42], ground: [0.05, 0.06, 0.12],
        shadow_tint: [0.60, 0.70, 1.0], shadow_saturation: 1.2,
        rim_intensity: 1.2, rim_spread: 0.50, rim_color: [0.55, 0.70, 1.0, 1.0]),
    rig!("interior", crate::icons::LAMP,
        az: -15.0, el: 52.0, kelvin: 3800.0, intensity: 0.85, ambient: 1.2,
        sky: [0.85, 0.78, 0.68], ground: [0.50, 0.42, 0.36],
        shadow_tint: [1.0, 0.90, 0.90], shadow_saturation: 0.9,
        rim_intensity: 0.5, rim_spread: 0.40, rim_color: [1.0, 0.85, 0.70, 1.0]),
    rig!("studio", crate::icons::APERTURE,
        az: -35.0, el: 30.0, kelvin: 6500.0, intensity: 1.1, ambient: 1.7,
        sky: [0.80, 0.82, 0.88], ground: [0.45, 0.45, 0.48],
        shadow_tint: [1.0, 1.0, 1.0], shadow_saturation: 1.0,
        rim_intensity: 1.2, rim_spread: 0.42, rim_color: [0.85, 0.92, 1.0, 1.0]),
];

pub fn rig_by_id(id: &str) -> Option<&'static RigSpec> {
    RIGS.iter().find(|r| r.id == id)
}

impl RigSpec {
    /// Cor da luz do rig (sRGB 0..1).
    pub fn light_color(&self) -> [f32; 3] {
        kelvin_to_rgb(self.kelvin)
    }

    /// Direção `L` do sol do rig.
    pub fn direction(&self) -> glam::Vec3 {
        dir_from_az_el(self.az, self.el)
    }

    /// Faixas que o núcleo/shader aceitam (usado pelo teste do catálogo).
    pub fn is_valid(&self) -> bool {
        let unit = |c: &[f32]| c.iter().all(|v| (0.0..=1.0).contains(v));
        (0.0..=3.0).contains(&self.intensity)
            && (0.0..=2.0).contains(&self.ambient)
            && unit(&self.sky)
            && unit(&self.ground)
            && unit(&self.shadow_tint)
            && (0.0..=2.0).contains(&self.shadow_saturation)
            && (0.0..=3.0).contains(&self.rim_intensity)
            && (0.05..=0.95).contains(&self.rim_spread)
            && unit(&self.rim_color)
    }
}

/// O comando que aplica o rig ao projeto: `SetLight` (key + ambiente + tinta de sombra) e
/// `SetMaterialParams` do rim do personagem, juntos num `Batch` = UM passo de desfazer.
/// Não toca nas configurações de sombra projetada (decisão do usuário).
pub fn rig_command(project: &ProjectState, rig: &RigSpec) -> Command {
    let light_id = project.scene.lights.first().map(|slot| slot.light_id.clone());
    Command::Batch {
        commands: vec![
            Command::SetLight {
                light_id,
                direction: Some(rig.direction().to_array()),
                color: Some(rig.light_color()),
                intensity: Some(rig.intensity),
                shadow_color: Some(rig.shadow_tint),
                ambient_intensity: Some(rig.ambient),
                shadow_saturation: Some(rig.shadow_saturation),
                ambient_sky: Some(rig.sky),
                ambient_ground: Some(rig.ground),
                shadow: None,
            },
            Command::SetMaterialParams {
                material_id: None,
                patch: MaterialPatch {
                    rim_intensity: Some(rig.rim_intensity),
                    rim_spread: Some(rig.rim_spread),
                    rim_color: Some(rig.rim_color),
                    ..MaterialPatch::default()
                },
            },
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anigo_session::CoreSession;

    #[test]
    fn catalog_has_the_seven_rigs_with_unique_ids_and_valid_ranges() {
        let ids: std::collections::BTreeSet<_> = RIGS.iter().map(|r| r.id).collect();
        assert_eq!(ids.len(), 7);
        for want in ["three_point", "rim", "day", "golden_hour", "night", "interior", "studio"] {
            assert!(rig_by_id(want).is_some(), "falta o rig {want}");
        }
        for r in &RIGS {
            assert!(r.is_valid(), "{} fora das faixas", r.id);
            assert!((r.direction().length() - 1.0).abs() < 1e-5);
        }
        assert!(rig_by_id("nope").is_none());
    }

    #[test]
    fn rim_rig_puts_the_light_behind_and_golden_hour_is_low_and_warm() {
        let rim = rig_by_id("rim").unwrap();
        assert!(rim.direction().z < -0.5, "contraluz: luz atrás do personagem (−Z)");
        let golden = rig_by_id("golden_hour").unwrap();
        assert!(golden.el < 20.0, "pôr do sol: sol baixo");
        let c = golden.light_color();
        assert!(c[0] > c[1] && c[1] > c[2], "luz quente: R > G > B ({c:?})");
        let night = rig_by_id("night").unwrap();
        let n = night.light_color();
        assert!(n[2] >= n[0], "noite: luz fria");
        assert!(night.intensity < rig_by_id("day").unwrap().intensity);
    }

    /// Contraste luz/sombra de um rig para o material padrão do personagem, pela MESMA conta do shader
    /// (luz = base × cor × intensidade; sombra = cor de sombra × tinta × hemisfério(N·up = 0,5) × ambiente;
    /// sem hue-shift nem AO). O ambiente MULTIPLICA o tom da sombra, então "fill 3:1" é um ambiente ~1,5.
    fn contrast(r: &RigSpec) -> f32 {
        let lin = |c: f32| if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) };
        let lum = |c: [f32; 3]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
        let m = anigo_core::StylizedMaterial::default();
        let light = r.light_color();
        let lit = lum([0, 1, 2].map(|i| lin(m.base_color[i]) * lin(light[i]) * r.intensity));
        let shadow = lum([0, 1, 2].map(|i| lin(m.shade_color[i]) * lin(r.shadow_tint[i]) * (lin(r.sky[i]) + lin(r.ground[i])) * 0.5 * r.ambient));
        lit / shadow
    }

    #[test]
    fn three_point_has_about_a_3_to_1_key_to_fill_contrast() {
        let r = rig_by_id("three_point").unwrap();
        let c = contrast(r);
        assert!((2.5..=4.5).contains(&c), "fill 3:1 → contraste {c}");
        assert!((15.0..=70.0).contains(&r.az.abs()), "key entre 15° e 70° da câmera");
        // interior é chapado (baixo contraste); contraluz e noite são contrastados
        assert!(contrast(rig_by_id("interior").unwrap()) < c);
        assert!(contrast(rig_by_id("rim").unwrap()) > c);
        for rig in &RIGS {
            assert!(contrast(rig) > 1.5, "{}: sombra tão clara quanto a luz", rig.id);
        }
    }

    #[test]
    fn applying_a_rig_is_a_single_undo_step_and_undo_restores_everything() {
        let mut core = CoreSession::new();
        let before = core.document().unwrap();
        for rig in &RIGS {
            let cmd = rig_command(core.project(), rig);
            core.apply(cmd).expect("rig aplica");
            let light = &core.project().scene.lights[0].light;
            assert_eq!(light.direction, rig.direction().to_array());
            assert_eq!(light.ambient_intensity, rig.ambient);
            assert_eq!(light.color, rig.light_color());
            let mat = &core.project().character_material().unwrap().material;
            assert_eq!(mat.rim_intensity, rig.rim_intensity);
            assert_eq!(mat.rim_color, rig.rim_color);
            core.undo().expect("um undo desfaz o rig inteiro");
            assert_eq!(core.document().unwrap(), before, "undo do rig {} não restaurou o documento", rig.id);
        }
        // redo refaz
        let cmd = rig_command(core.project(), rig_by_id("night").unwrap());
        core.apply(cmd).unwrap();
        let after = core.document().unwrap();
        core.undo().unwrap();
        core.redo().unwrap();
        assert_eq!(core.document().unwrap(), after);
    }

    #[test]
    fn the_rig_command_leaves_the_shadow_settings_alone() {
        let mut core = CoreSession::new();
        core.apply(Command::SetLight {
            light_id: None, direction: None, color: None, intensity: None, shadow_color: None,
            ambient_intensity: None, shadow_saturation: None, ambient_sky: None, ambient_ground: None,
            shadow: Some(anigo_core::LightShadow { enabled: true, strength: 0.7, ..Default::default() }),
        })
        .unwrap();
        let cmd = rig_command(core.project(), rig_by_id("day").unwrap());
        core.apply(cmd).unwrap();
        let sh = core.project().scene.lights[0].light.shadow;
        assert!(sh.enabled && (sh.strength - 0.7).abs() < 1e-6);
    }
}
