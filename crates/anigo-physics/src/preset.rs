//! Presets de parâmetros típicos de anime (cabelo curto/longo, saia, capa, acessório).
//!
//! ATENÇÃO: são pontos de partida coerentes com as faixas do VRoid (rigidez 0..1, gravidade 0..1)
//! e com a escala de compliance do XPBD, calibrados só por testes numéricos (estabilidade, sem
//! esticar, sem atravessar). A "sensação" visual NÃO foi validada: ajuste olhando o personagem.

use serde::{Deserialize, Serialize};

use crate::cloth::ClothParams;
use crate::hair::HairParams;
use crate::xpbd::XpbdConfig;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PhysicsPreset {
    HairShort,
    HairLong,
    Skirt,
    Cape,
    Accessory,
}

impl PhysicsPreset {
    pub const ALL: [PhysicsPreset; 5] =
        [PhysicsPreset::HairShort, PhysicsPreset::HairLong, PhysicsPreset::Skirt, PhysicsPreset::Cape, PhysicsPreset::Accessory];

    pub fn name(self) -> &'static str {
        match self {
            PhysicsPreset::HairShort => "hair_short",
            PhysicsPreset::HairLong => "hair_long",
            PhysicsPreset::Skirt => "skirt",
            PhysicsPreset::Cape => "cape",
            PhysicsPreset::Accessory => "accessory",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.name() == name)
    }

    /// Parâmetros de cabelo/acessório. Para saia e capa devolve `None` (use [`Self::cloth_params`]).
    pub fn hair_params(self) -> Option<HairParams> {
        match self {
            PhysicsPreset::HairShort => Some(HairParams {
                stiffness_root: 0.98,
                stiffness_tip: 0.70,
                stiffness_power: 1.0,
                bending_compliance: 1e-5,
                damping_root: 10.0,
                damping_tip: 5.0,
                gravity_scale: 0.6,
                tether_stretch: 0.02,
                particle_radius: 0.010,
                ..Default::default()
            }),
            PhysicsPreset::HairLong => Some(HairParams {
                stiffness_root: 0.97,
                stiffness_tip: 0.35,
                stiffness_power: 1.2,
                bending_compliance: 3e-5,
                damping_root: 8.0,
                damping_tip: 2.5,
                gravity_scale: 0.8,
                tether_stretch: 0.03,
                particle_radius: 0.012,
                ..Default::default()
            }),
            PhysicsPreset::Accessory => Some(HairParams {
                stiffness_root: 0.85,
                stiffness_tip: 0.20,
                stiffness_power: 1.0,
                bending_compliance: 1e-4,
                tip_mass_scale: 1.5,
                damping_root: 4.0,
                damping_tip: 1.5,
                gravity_scale: 1.0,
                tether_stretch: 0.05,
                particle_radius: 0.015,
                ..Default::default()
            }),
            PhysicsPreset::Skirt | PhysicsPreset::Cape => None,
        }
    }

    pub fn cloth_params(self) -> Option<ClothParams> {
        match self {
            PhysicsPreset::Skirt => Some(ClothParams::default()),
            PhysicsPreset::Cape => Some(ClothParams {
                angle_stiffness_root: 0.90,
                angle_stiffness_tip: 0.10,
                max_distance_root: 0.02,
                max_distance_tip: 0.60,
                backstop: f32::INFINITY,
                damping_root: 3.0,
                damping_tip: 1.0,
                gravity_scale: 1.0,
                ..Default::default()
            }),
            _ => None,
        }
    }

    /// Configuração do solver (sub-passos/iterações) adequada ao preset.
    pub fn xpbd_config(self) -> XpbdConfig {
        match self {
            PhysicsPreset::HairShort | PhysicsPreset::Accessory => XpbdConfig { substeps: 5, ..Default::default() },
            PhysicsPreset::HairLong => XpbdConfig { substeps: 6, ..Default::default() },
            // 2 iteracoes por sub-passo reduzem pela metade o estiramento quando uma perna empurra a saia
            PhysicsPreset::Skirt | PhysicsPreset::Cape => XpbdConfig { substeps: 6, iterations: 2, ..Default::default() },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn todo_preset_tem_parametros_e_nome_unico() {
        let mut names = std::collections::HashSet::new();
        for p in PhysicsPreset::ALL {
            assert!(names.insert(p.name()));
            assert_eq!(PhysicsPreset::from_name(p.name()), Some(p));
            assert!(p.hair_params().is_some() ^ p.cloth_params().is_some(), "{p:?}: exatamente um tipo");
            assert!(p.xpbd_config().substeps >= 1);
        }
        assert!(PhysicsPreset::from_name("nope").is_none());
    }

    #[test]
    fn presets_serializam_em_json() {
        let json = serde_json::to_string(&PhysicsPreset::HairLong).unwrap();
        assert_eq!(json, "\"hair_long\"");
        let back: PhysicsPreset = serde_json::from_str(&json).unwrap();
        assert_eq!(back, PhysicsPreset::HairLong);
    }

    #[test]
    fn rigidez_de_cabelo_longo_decresce_da_raiz_a_ponta() {
        for p in [PhysicsPreset::HairShort, PhysicsPreset::HairLong, PhysicsPreset::Accessory] {
            let h = p.hair_params().unwrap();
            assert!(h.stiffness_root > h.stiffness_tip, "{p:?}");
        }
        let long = PhysicsPreset::HairLong.hair_params().unwrap();
        let short = PhysicsPreset::HairShort.hair_params().unwrap();
        assert!(long.stiffness_tip < short.stiffness_tip, "cabelo longo balança mais na ponta");
    }
}
