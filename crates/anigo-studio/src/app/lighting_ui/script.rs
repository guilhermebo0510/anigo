//! Cenários de captura do Iluminação (`--scenario iluminacao-*`): montam uma cena DE VERDADE (comandos do núcleo:
//! piso, parede, adereços e o personagem) e aplicam luz/rig/sombra pelos MESMOS comandos da UI, levando a tela a
//! um estado conhecido. O screenshot só sai quando a geometria assentou.
//!
//! Nomes: `iluminacao-base`, `iluminacao-luz-<az>-<el>-<intensidade×100>` (negativo com `n`: `n60`),
//! `iluminacao-rig-<id>`, `iluminacao-sombra`, `iluminacao-sombra-suave`, `iluminacao-rosto`,
//! `iluminacao-camera`, `iluminacao-ambiente`, `iluminacao-extras`.

use super::{LightPatch, Section, StudioApp};
use crate::lighting;
#[cfg(test)]
use crate::lighting::rigs;
use crate::scene::runtime::GeomState;
use anigo_core::command::Command;
use anigo_core::ids::NodeId;
use anigo_core::level::{self, NodeAttrs, PrimitiveShape};
use anigo_core::scene::LightShadow;
use anigo_core::{NodeOverrides, Transform};
use glam::{Quat, Vec3};
use std::collections::BTreeSet;

/// Cenários fixos (os parametrizados `iluminacao-luz-*` e `iluminacao-rig-*` seguem o padrão dos nomes).
#[allow(dead_code)]
pub const LIGHT_SCENARIOS: &[&str] = &[
    "iluminacao-base",
    "iluminacao-luz-n60-35-100",
    "iluminacao-luz-60-35-100",
    "iluminacao-luz-0-78-100",
    "iluminacao-luz-170-30-120",
    "iluminacao-luz-60-35-35",
    "iluminacao-rig-three_point",
    "iluminacao-rig-rim",
    "iluminacao-rig-day",
    "iluminacao-rig-golden_hour",
    "iluminacao-rig-night",
    "iluminacao-rig-interior",
    "iluminacao-rig-studio",
    "iluminacao-sombra",
    "iluminacao-sombra-suave",
    "iluminacao-rosto",
    "iluminacao-rosto-sem",
    "iluminacao-camera",
    "iluminacao-ambiente",
    "iluminacao-extras",
];

pub(crate) fn is_light_scenario(name: &str) -> bool {
    name.starts_with("iluminacao-")
}

pub(crate) struct Script {
    pub name: String,
    stage: u32,
    stage_frame: u64,
    frame: u64,
    done: bool,
}

impl Script {
    pub fn is_done(&self) -> bool {
        self.done
    }
}

/// `n60` → −60, `35` → 35.
fn parse_signed(token: &str) -> Option<f32> {
    match token.strip_prefix('n') {
        Some(rest) => rest.parse::<f32>().ok().map(|v| -v),
        None => token.parse::<f32>().ok(),
    }
}

/// `iluminacao-luz-<az>-<el>-<int×100>` → (azimute, elevação, intensidade).
pub(crate) fn parse_light_scenario(name: &str) -> Option<(f32, f32, f32)> {
    let rest = name.strip_prefix("iluminacao-luz-")?;
    let mut it = rest.split('-');
    let az = parse_signed(it.next()?)?;
    let el = parse_signed(it.next()?)?;
    let int = parse_signed(it.next()?)? / 100.0;
    it.next().is_none().then_some((az, el, int))
}

impl StudioApp {
    pub(crate) fn light_start_script(&mut self, name: &str) {
        self.ws = crate::workspace::WorkspaceId::Iluminacao;
        self.light_ui.script = Some(Script { name: name.to_string(), stage: 0, stage_frame: 0, frame: 0, done: false });
    }

    /// Piso, parede, orbe e caixa como UM comando (um passo de desfazer): o "cenário" para ver luz e sombra.
    fn light_set_commands(&self, with_floor: bool) -> Command {
        let mut taken: BTreeSet<NodeId> = BTreeSet::new();
        let mut cmds = Vec::new();
        let mut add = |shape: PrimitiveShape, label: &str, t: Transform, colors: Option<([f32; 4], [f32; 4])>| {
            let (mut cmd, id) = level::plan_place_primitive(self.core.project(), shape, label, t, &taken);
            taken.insert(id);
            if let (Some((base, shade)), Command::AddNode { attrs, .. }) = (colors, &mut cmd) {
                let a = attrs.get_or_insert_with(NodeAttrs::default);
                a.overrides = NodeOverrides { base_color: Some(base), shade_color: Some(shade), outline_width: None };
            }
            cmds.push(cmd);
        };
        if with_floor {
            add(PrimitiveShape::Plane, "Piso", Transform { scale: Vec3::new(18.0, 1.0, 13.0), ..Default::default() }, Some(([0.78, 0.80, 0.86, 1.0], [0.42, 0.45, 0.62, 1.0])));
            add(PrimitiveShape::Cube, "Parede", Transform { translation: Vec3::new(0.0, 2.0, -3.4), scale: Vec3::new(18.0, 4.0, 0.3), ..Default::default() }, Some(([0.86, 0.80, 0.78, 1.0], [0.50, 0.42, 0.52, 1.0])));
        }
        add(PrimitiveShape::Sphere, "Orbe", Transform { translation: Vec3::new(1.9, 0.55, 0.6), scale: Vec3::splat(1.1), ..Default::default() }, Some(([0.95, 0.62, 0.55, 1.0], [0.55, 0.30, 0.38, 1.0])));
        add(
            PrimitiveShape::Cube,
            "Caixa",
            Transform { translation: Vec3::new(-2.0, 0.45, 0.2), rotation: Quat::from_rotation_y(0.5), scale: Vec3::splat(0.9) },
            Some(([0.55, 0.78, 0.65, 1.0], [0.28, 0.42, 0.45, 1.0])),
        );
        Command::Batch { commands: cmds }
    }

    /// Botão "Adicionar piso de prova": só um plano sob o personagem (UM passo de desfazer).
    pub(crate) fn light_add_test_floor(&mut self) {
        let taken = BTreeSet::new();
        let (mut cmd, _) = level::plan_place_primitive(
            self.core.project(),
            PrimitiveShape::Plane,
            "Piso de prova",
            Transform { scale: Vec3::new(14.0, 1.0, 10.0), ..Default::default() },
            &taken,
        );
        if let Command::AddNode { attrs, .. } = &mut cmd {
            let a = attrs.get_or_insert_with(NodeAttrs::default);
            a.overrides = NodeOverrides { base_color: Some([0.78, 0.80, 0.86, 1.0]), shade_color: Some([0.42, 0.45, 0.62, 1.0]), outline_width: None };
        }
        self.light_apply(cmd);
    }

    /// Botão "Adicionar volume de cabelo (prova)": um elipsoide-franja sobre a cabeça, para ver a sombra do
    /// cabelo no rosto. É um PROXY (primitiva) — o cabelo do workspace Personagem ainda não é malha de cena.
    pub(crate) fn light_add_hair_proxy(&mut self) {
        let Some(b) = self.scene_ui.runtime.bounds_of(&NodeId::canonical_character()) else {
            self.light_say(crate::i18n::tr("light.no_geometry"));
            return;
        };
        let (c, size) = (b.center(), b.max - b.min);
        // cabeça ≈ 1/7 da altura (o manequim canônico: 0,26 m de cabeça em 1,82 m). O volume é uma "touca" com
        // franja: elipsoide achatado que cobre só o topo da cabeça e avança além da testa, para sombrear testa e olhos.
        let head_h = size.y / 7.0;
        let top = b.max.y;
        let taken = BTreeSet::new();
        let (mut cmd, _) = level::plan_place_primitive(
            self.core.project(),
            PrimitiveShape::Sphere,
            "Cabelo (prova)",
            Transform {
                translation: Vec3::new(c.x, top - head_h * 0.20, c.z + head_h * 0.12),
                scale: Vec3::new(head_h * 0.98, head_h * 0.42, head_h * 1.25),
                ..Default::default()
            },
            &taken,
        );
        if let Command::AddNode { attrs, .. } = &mut cmd {
            let a = attrs.get_or_insert_with(NodeAttrs::default);
            a.overrides = NodeOverrides { base_color: Some([0.20, 0.14, 0.30, 1.0]), shade_color: Some([0.09, 0.07, 0.16, 1.0]), outline_width: None };
        }
        self.light_apply(cmd);
    }

    fn light_script_light(&mut self, patch: LightPatch) {
        let id = self.light_view().id;
        self.light_apply(patch.into_command(id));
    }

    fn light_script_shadow(&mut self, hardness: f32, resolution: u32) {
        let shadow = LightShadow { enabled: true, hardness, resolution, ..LightShadow::default() };
        self.light_script_light(LightPatch { shadow: Some(shadow), ..Default::default() });
    }

    /// Máquina de estados dos cenários (chamada a cada quadro do workspace).
    pub(crate) fn light_script_tick(&mut self) {
        let Some(mut run) = self.light_ui.script.take() else { return };
        if run.done {
            self.light_ui.script = Some(run);
            return;
        }
        run.frame += 1;
        let waited = run.frame.saturating_sub(run.stage_frame);
        let mut next = false;
        match run.stage {
            0 => {
                if self.gpu.is_some() && run.frame > 4 {
                    self.core.set_history_limit(400);
                    next = true;
                }
            }
            1 => {
                // monta o conjunto (um comando) e deixa o Cenário carregar as primitivas
                let name = run.name.clone();
                let with_floor = !name.starts_with("iluminacao-rosto");
                let cmd = self.light_set_commands(with_floor);
                self.light_apply(cmd);
                self.light_ui.message.clear();
                self.light_ui.framed = false;
                next = true;
            }
            2 => {
                let loading = self.scene_ui.runtime.geom.values().any(|g| matches!(g, GeomState::Loading)) || self.scene_ui.cache.loading_count() > 0;
                if !loading && waited > 6 && self.scene_ui.runtime.bounds_of(&NodeId::canonical_character()).is_some() {
                    next = true;
                }
            }
            3 => {
                let name = run.name.clone();
                self.light_configure(&name);
                next = true;
            }
            _ => {
                if waited > 8 {
                    run.done = true;
                }
            }
        }
        if next {
            run.stage += 1;
            run.stage_frame = run.frame;
        }
        if run.frame > 3000 {
            run.done = true;
        }
        self.light_ui.script = Some(run);
    }

    fn light_configure(&mut self, name: &str) {
        self.light_frame_character();
        // enquadramento padrão dos cenários: um pouco de lado para ver o volume e a sombra no piso
        self.light_ui.camera.yaw = 0.35;
        self.light_ui.camera.pitch = 0.16;
        if let Some((az, el, int)) = parse_light_scenario(name) {
            let d = lighting::dir_from_az_el(az, el);
            self.light_script_light(LightPatch { direction: Some(d.to_array()), intensity: Some(int), ..Default::default() });
            self.light_ui.focus = Some(Section::Sun);
            return;
        }
        if let Some(id) = name.strip_prefix("iluminacao-rig-") {
            self.light_apply_rig(id);
            self.light_ui.focus = Some(Section::Rigs);
            return;
        }
        match name {
            "iluminacao-sombra" | "iluminacao-sombra-suave" => {
                let d = lighting::dir_from_az_el(-52.0, 36.0);
                self.light_script_light(LightPatch { direction: Some(d.to_array()), intensity: Some(1.1), ..Default::default() });
                self.light_script_shadow(if name.ends_with("suave") { 0.05 } else { 0.95 }, 2048);
                self.light_ui.focus = Some(Section::Shadows);
            }
            "iluminacao-rosto" | "iluminacao-rosto-sem" => {
                // close na cabeça; o bloco de cabelo (proxy) projeta na testa/olhos com a luz alta e de frente
                self.light_add_hair_proxy();
                let d = lighting::dir_from_az_el(18.0, 52.0);
                self.light_script_light(LightPatch { direction: Some(d.to_array()), intensity: Some(1.15), ..Default::default() });
                if name == "iluminacao-rosto" {
                    self.light_script_shadow(0.9, 4096);
                }
                if let Some(b) = self.scene_ui.runtime.bounds_of(&NodeId::canonical_character()) {
                    let head_h = (b.max.y - b.min.y) / 7.0;
                    let cam = &mut self.light_ui.camera;
                    cam.target = Vec3::new(b.center().x, b.max.y - head_h * 0.6, b.center().z);
                    cam.distance = 1.15;
                    cam.yaw = 0.22;
                    cam.pitch = 0.05;
                }
                self.light_ui.focus = Some(Section::Shadows);
            }
            "iluminacao-camera" => {
                self.light_set_follow(true);
                self.light_ui.camera.yaw = 1.0;
                self.light_ui.focus = Some(Section::Rigs);
            }
            "iluminacao-ambiente" => {
                self.light_script_light(LightPatch {
                    ambient_intensity: Some(1.15),
                    ambient_sky: Some([0.45, 0.62, 1.0]),
                    ambient_ground: Some([0.85, 0.55, 0.35]),
                    ..Default::default()
                });
                self.light_ui.focus = Some(Section::Ambient);
            }
            "iluminacao-extras" => {
                self.light_ui.focus = Some(Section::Extra);
            }
            "iluminacao-base" => {
                self.light_ui.focus = Some(Section::Rigs);
            }
            _ => {
                self.light_ui.focus = Some(Section::Sun);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scenario_names_are_recognised_and_unique() {
        let mut seen = BTreeSet::new();
        for n in LIGHT_SCENARIOS {
            assert!(is_light_scenario(n) && seen.insert(*n), "{n}");
        }
        assert!(!is_light_scenario("cenario-montado"));
    }

    #[test]
    fn light_scenarios_parse_signed_numbers() {
        assert_eq!(parse_light_scenario("iluminacao-luz-n60-35-100"), Some((-60.0, 35.0, 1.0)));
        assert_eq!(parse_light_scenario("iluminacao-luz-170-30-120"), Some((170.0, 30.0, 1.2)));
        assert_eq!(parse_light_scenario("iluminacao-luz-0-n10-35"), Some((0.0, -10.0, 0.35)));
        assert_eq!(parse_light_scenario("iluminacao-luz-1-2"), None);
        assert_eq!(parse_light_scenario("iluminacao-luz-1-2-3-4"), None);
        assert_eq!(parse_light_scenario("iluminacao-base"), None);
    }

    #[test]
    fn every_fixed_scenario_is_either_known_or_parametric() {
        for n in LIGHT_SCENARIOS {
            let known = parse_light_scenario(n).is_some()
                || n.strip_prefix("iluminacao-rig-").is_some_and(|id| rigs::rig_by_id(id).is_some())
                || ["iluminacao-base", "iluminacao-sombra", "iluminacao-sombra-suave", "iluminacao-rosto", "iluminacao-rosto-sem", "iluminacao-camera", "iluminacao-ambiente", "iluminacao-extras"].contains(n);
            assert!(known, "cenário sem tratamento: {n}");
        }
    }
}
