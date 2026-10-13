//! Cenários de captura do Shading (`--scenario shading-*`): levam o workspace a um estado conhecido
//! usando as MESMAS rotas da UI (`shading_apply_now`, comandos do núcleo, histórico da Modelagem), para
//! screenshots determinísticos e para o smoke do CI.

use super::{FaceMap, PresetTab, Source, StudioApp, Target};
use crate::modeling::PrimitiveKind;
use crate::shading::ball::PreviewMode;
use crate::shading::params::{self, Group};
use crate::shading::presets::{self, BUILT_IN};
use anigo_core::level::{self, PrimitiveShape};
use anigo_core::{StylizedMaterial, Transform};
use glam::Vec3;
use std::collections::BTreeSet;

#[allow(dead_code)]
pub const SHADING_SCENARIOS: &[&str] = &[
    "shading-ball",
    "shading-rampa",
    "shading-rim",
    "shading-contorno",
    "shading-face-sdf",
    "shading-face-sdf-frente",
    "shading-face-sdf-espelho",
    "shading-ab",
    "shading-paleta",
    "shading-presets",
    "shading-cenario",
    "shading-modelagem",
    "shading-smoke",
];

pub(crate) fn is_shading_scenario(name: &str) -> bool {
    name.starts_with("shading-")
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

/// Material de meio-tom das capturas (o padrão claro "pele" estoura em branco e esconde o efeito).
fn mid_tone() -> StylizedMaterial {
    StylizedMaterial {
        base_color: [0.42, 0.60, 0.86, 1.0],
        shade_color: [0.18, 0.22, 0.46, 1.0],
        rim_color: [1.0, 0.85, 0.6, 1.0],
        outline_color: [0.05, 0.05, 0.12, 1.0],
        spec_intensity: 0.4,
        rim_intensity: 0.0,
        outline_width: 0.0,
        ..Default::default()
    }
}

impl StudioApp {
    pub(crate) fn shading_start_script(&mut self, name: &str) {
        self.ws = crate::workspace::WorkspaceId::Shading;
        self.shading_ui.script = Some(Script { name: name.to_string(), stage: 0, stage_frame: 0, frame: 0, done: false });
    }

    fn shading_set_material(&mut self, m: StylizedMaterial) {
        self.shading_validate_target();
        self.shading_apply_now(m);
    }

    fn shading_current(&self) -> StylizedMaterial {
        self.shading_ui.target.as_ref().and_then(|t| self.shading_read(t)).unwrap_or_default()
    }

    /// Máquina de estados dos cenários (chamada a cada quadro do workspace).
    pub(crate) fn shading_script_tick(&mut self) {
        let Some(mut run) = self.shading_ui.script.take() else { return };
        if run.done {
            self.shading_ui.script = Some(run);
            return;
        }
        run.frame += 1;
        let waited = run.frame.saturating_sub(run.stage_frame);
        let gpu_ready = self.gpu.as_ref().is_some_and(|g| g.cel.is_some());
        let lib_ready = self.lib_ws.as_ref().is_some_and(|w| w.is_ready());
        let mut next = false;
        match run.stage {
            0 => {
                if gpu_ready && lib_ready {
                    next = true;
                }
            }
            1 => {
                let name = run.name.clone();
                self.shading_run_scenario(&name);
                next = true;
            }
            2 => {
                // deixa miniaturas e a geometria assentarem
                if waited > 14 {
                    run.done = true;
                }
            }
            _ => {}
        }
        if next {
            run.stage += 1;
            run.stage_frame = run.frame;
        }
        self.shading_ui.script = Some(run);
    }

    fn shading_run_scenario(&mut self, name: &str) {
        self.shading_ui.source = Source::Personagem;
        self.shading_ui.target = self.shading_character_target();
        self.shading_ui.mode = PreviewMode::Both;
        self.shading_ui.framed = false;
        self.shading_ui.rig.follows_camera = false;
        self.shading_ui.rig.azimuth_deg = -50.0;
        self.shading_ui.rig.elevation_deg = 28.0;
        // câmera de frente: o SDF facial e o terminador ficam legíveis
        self.shading_ui.camera.yaw = 0.0;
        self.shading_ui.camera.pitch = 0.06;
        self.core.set_history_limit(400);
        match name {
            "shading-ball" => {
                let mut m = mid_tone();
                m.rim_intensity = 0.8;
                m.outline_width = 0.006;
                self.shading_set_material(m);
                self.shading_ui.mode = PreviewMode::Ball;
            }
            "shading-rampa" => {
                let mut m = mid_tone();
                m.npr.ramp_blend = 1.0;
                m.npr.ramp_row = 2.0;
                m.npr.ramp_custom.set_stops(2, false, &[(0.0, 0.0), (0.38, 0.35), (0.5, 0.55), (0.66, 1.0)]);
                self.shading_set_material(m);
                self.shading_ui.mode = PreviewMode::Ball;
                self.shading_ui.focus_group = Some(Group::Ramp);
                self.shading_ui.ramp_sel = Some(1);
            }
            "shading-rim" => {
                let mut m = mid_tone();
                m.rim_intensity = 2.2;
                m.rim_spread = 0.55;
                m.rim_color = [1.0, 0.8, 0.45, 1.0];
                self.shading_set_material(m);
                self.shading_ui.mode = PreviewMode::Ball;
                self.shading_ui.rig.azimuth_deg = 140.0; // contraluz: o rim aparece
                self.shading_ui.rig.elevation_deg = 18.0;
                self.shading_ui.focus_group = Some(Group::Rim);
            }
            "shading-contorno" => {
                let mut m = mid_tone();
                m.outline_width = 0.012;
                m.npr.outline_smooth_normals = 1.0;
                m.npr.outline_distance_falloff = 0.8;
                m.npr.outline_taper = 0.6;
                m.npr.outline_ink_blend = 0.8;
                m.rim_intensity = 0.4;
                self.shading_set_material(m);
                self.shading_ui.mode = PreviewMode::Both;
                self.shading_ui.focus_group = Some(Group::Outline);
            }
            "shading-face-sdf" | "shading-face-sdf-frente" | "shading-face-sdf-espelho" => {
                let mut m = mid_tone();
                m.face_sdf_enabled = true;
                m.shadow_threshold = 0.2;
                m.spec_intensity = 0.0;
                m.face_shadow_smoothness = 0.04;
                if name != "shading-face-sdf" && name != "shading-face-sdf-frente" {
                    m.npr.face_sdf_mirror_center = 0.25;
                }
                self.shading_set_material(m);
                self.shading_install_face_map(FaceMap::Procedural);
                self.shading_ui.mode = PreviewMode::Ball;
                self.shading_ui.rig.elevation_deg = 0.0;
                self.shading_ui.rig.azimuth_deg = match name {
                    "shading-face-sdf-frente" => 0.0,
                    "shading-face-sdf-espelho" => -80.0,
                    _ => 80.0,
                };
                self.shading_ui.focus_group = Some(Group::Face);
            }
            "shading-ab" => {
                let before = mid_tone();
                self.shading_set_material(before.clone());
                self.shading_ui.snapshot_a = Some(before);
                self.shading_ui.compare = true;
                let mut after = self.shading_current();
                after.npr.ramp_blend = 1.0;
                after.npr.ramp_row = 2.0;
                after.rim_intensity = 1.4;
                after.outline_width = 0.008;
                self.shading_apply_now(after);
                self.shading_ui.mode = PreviewMode::Ball;
            }
            "shading-paleta" => {
                self.shading_set_material(mid_tone());
                self.shading_ui.palette_open = true;
                let base = [0.42, 0.60, 0.86];
                for c in crate::shading::palette::harmony_colors(base, crate::shading::palette::Harmony::Triad) {
                    self.shading_ui.palette.add(c);
                }
                self.shading_ui.harmony = crate::shading::palette::Harmony::Analogous;
                self.shading_ui.mode = PreviewMode::Ball;
            }
            "shading-presets" => {
                self.shading_set_material(mid_tone());
                if let Some(lib) = self.lib_ws.as_ref().and_then(|w| w.library().cloned()) {
                    for p in BUILT_IN.iter().take(4) {
                        let m = (p.build)();
                        let _ = presets::save_preset(&lib, &m.name.clone(), &m, &Default::default());
                    }
                }
                self.shading_ui.preset_rows_for = None;
                self.shading_ui.preset_tab = PresetTab::Library;
                self.shading_ui.mode = PreviewMode::Ball;
            }
            "shading-cenario" => self.shading_scenario_scene(),
            "shading-modelagem" => self.shading_scenario_modeling(),
            _ => {
                // smoke: material de meio-tom + rim para o xdotool ter o que arrastar
                let mut m = mid_tone();
                m.rim_intensity = 0.5;
                self.shading_set_material(m);
                self.shading_ui.mode = PreviewMode::Ball;
                self.shading_ui.focus_group = Some(Group::Rim);
            }
        }
        self.core.set_history_limit(400);
    }

    /// Cenário: três props com materiais próprios (instâncias) e o primeiro como alvo.
    fn shading_scenario_scene(&mut self) {
        let mut ids = Vec::new();
        for (shape, label, x) in [(PrimitiveShape::Sphere, "Esfera", -1.4), (PrimitiveShape::Cube, "Caixa", 0.0), (PrimitiveShape::Cylinder, "Coluna", 1.4)] {
            let t = Transform { translation: Vec3::new(x, 0.5, 0.0), ..Default::default() };
            let (cmd, id) = level::plan_place_primitive(self.core.project(), shape, label, t, &BTreeSet::new());
            let _ = self.core.apply(cmd);
            ids.push(id);
        }
        self.shading_ui.source = Source::Cenario;
        for (i, id) in ids.iter().enumerate() {
            self.shading_ui.target = Some(Target::Node(id.clone()));
            let mut m = (BUILT_IN[[0, 2, 4][i]].build)();
            m.name = self.shading_current().name;
            self.shading_apply_now(m);
        }
        self.shading_ui.target = Some(Target::Node(ids[0].clone()));
        self.shading_ui.mode = PreviewMode::Both;
        self.shading_ui.focus_group = Some(Group::Base);
    }

    /// Modelagem: três objetos com materiais completos pelo histórico da `ModelingSession`.
    fn shading_scenario_modeling(&mut self) {
        self.modeling.add_primitive(PrimitiveKind::QuadSphere, Vec3::new(2.4, 1.0, 0.0));
        self.modeling.add_primitive(PrimitiveKind::Torus, Vec3::new(-2.4, 1.0, 0.0));
        self.shading_ui.source = Source::Modelagem;
        let ids: Vec<u32> = self.modeling.doc.objects.iter().map(|o| o.id).collect();
        for (i, id) in ids.iter().enumerate() {
            self.shading_ui.target = Some(Target::Object(*id));
            let mut m = (BUILT_IN[[1, 3, 5][i % 3]].build)();
            m.name = format!("obj{id}");
            self.shading_apply_now(m);
        }
        self.shading_ui.target = ids.last().map(|id| Target::Object(*id));
        self.shading_ui.mode = PreviewMode::Both;
        let _ = params::PARAMS.len();
    }
}
