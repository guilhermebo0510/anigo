//! Cenários de captura do Render (`--scenario render-*`): montam uma cena DE VERDADE (comandos do núcleo, ativos
//! reais da Biblioteca sintética) e levam a UI a um estado conhecido. `render-fila` faz um render real (offscreen, thread
//! de GPU dedicada) numa pasta temporária e só termina o screenshot depois que o job acabou.

use super::{JobStatus, RenderTab, StudioApp};
use crate::scene::runtime::GeomState;
use anigo_core::command::Command;
use anigo_core::level;
use anigo_core::{AovSelection, NodeSource, OutputFormat, RenderOutputPatch, Transform};
use glam::{Quat, Vec3};
use std::collections::BTreeSet;

#[allow(dead_code)]
pub const RENDER_SCENARIOS: &[&str] = &["render-vazio", "render-camera", "render-saida", "render-passes", "render-transparente", "render-fila"];

pub(crate) fn is_render_scenario(name: &str) -> bool {
    name.starts_with("render-")
}

pub(crate) struct Script {
    pub name: String,
    stage: u32,
    stage_frame: u64,
    frame: u64,
    done: bool,
    /// Pasta temporária do render do cenário `render-fila` (apagada ao sair).
    pub temp_dir: Option<std::path::PathBuf>,
}

impl Script {
    pub fn is_done(&self) -> bool {
        self.done
    }
}

impl StudioApp {
    pub(crate) fn render_start_script(&mut self, name: &str) {
        self.ws = crate::workspace::WorkspaceId::Render;
        self.render_ui.script = Some(Script { name: name.to_string(), stage: 0, stage_frame: 0, frame: 0, done: false, temp_dir: None });
    }

    /// Apaga a pasta temporária do cenário (a menos que `ANIGO_SCENARIO_KEEP` esteja definida).
    pub(crate) fn render_cleanup(&mut self) {
        if std::env::var_os("ANIGO_SCENARIO_KEEP").is_some() {
            return;
        }
        if let Some(d) = self.render_ui.script.as_mut().and_then(|s| s.temp_dir.take()) {
            let _ = std::fs::remove_dir_all(d);
        }
    }

    fn set_tab(&mut self, tab: RenderTab) {
        let idx = match tab {
            RenderTab::Camera => 0,
            RenderTab::Passes => 1,
            RenderTab::Output => 2,
        };
        self.tool_idx.insert(crate::workspace::WorkspaceId::Render, idx);
    }

    /// Câmera 3/4 da Cenário + enquadramento na proporção da saída.
    fn script_aim_camera(&mut self) {
        self.scene_refresh();
        let c = self.scene_ui.camera;
        self.render_apply(Command::SetCamera { eye: Some(c.eye().to_array()), target: Some(c.target.to_array()), up: Some([0.0, 1.0, 0.0]), fov_degrees: Some(32.0), projection: None });
        self.render_frame(false);
    }

    /// Máquina de estados dos cenários (chamada a cada quadro do workspace).
    pub(crate) fn render_script_tick(&mut self, ctx: &egui::Context) {
        let Some(mut run) = self.render_ui.script.take() else { return };
        if run.done {
            self.render_ui.script = Some(run);
            return;
        }
        run.frame += 1;
        let ready = self.lib_ws.as_ref().is_some_and(|w| w.is_ready());
        let settled = self.lib_ws.as_ref().is_some_and(|w| w.is_settled());
        let waited = run.frame.saturating_sub(run.stage_frame);
        let name = run.name.clone();
        let mut next = false;
        match run.stage {
            0 => {
                if ready {
                    if let Some(w) = self.lib_ws.as_mut() {
                        w.prepare_scene_drawer();
                    }
                    self.core.set_history_limit(400);
                    self.scene_ui.history_limit_set = true;
                    next = true;
                }
            }
            1 => {
                if settled && waited > 3 {
                    next = true;
                }
            }
            2 => {
                match name.as_str() {
                    "render-vazio" => self.set_tab(RenderTab::Camera),
                    _ => {
                        self.build_set(name != "render-camera");
                        self.scene_ui.selection.clear();
                    }
                }
                next = true;
            }
            3 => {
                let loading = self.scene_ui.runtime.geom.values().any(|g| matches!(g, GeomState::Loading)) || self.scene_ui.cache.loading_count() > 0;
                if !loading && settled && waited > 6 {
                    self.script_scene_ready(ctx, &name, &mut run);
                    next = true;
                }
            }
            4 => {
                // render-fila: espera o job terminar
                if name == "render-fila" {
                    let finished = self.render_ui.jobs.last().is_some_and(|j| !matches!(j.status, JobStatus::Queued | JobStatus::Running));
                    if finished && waited > 6 {
                        next = true;
                    }
                } else if waited > 8 {
                    next = true;
                }
            }
            _ => {
                if waited > 6 {
                    run.done = true;
                }
            }
        }
        if next {
            run.stage += 1;
            run.stage_frame = run.frame;
        }
        if run.frame > 3500 {
            run.done = true;
        }
        self.render_ui.script = Some(run);
    }

    fn script_scene_ready(&mut self, ctx: &egui::Context, name: &str, run: &mut Script) {
        let patch = |p: RenderOutputPatch| Command::SetRenderOutput { patch: p };
        match name {
            "render-camera" => {
                self.set_tab(RenderTab::Camera);
                let (cmd, cam) = level::plan_place_camera(
                    self.core.project(),
                    "Câmera principal",
                    Transform { translation: Vec3::new(4.2, 1.7, 7.0), rotation: Quat::from_euler(glam::EulerRot::YXZ, 0.62, -0.08, 0.0), scale: Vec3::ONE },
                    NodeSource::Camera { focal_mm: 35.0, sensor_width_mm: 24.89, sensor_height_mm: 18.66 },
                    &BTreeSet::new(),
                );
                self.render_apply(cmd);
                self.render_ui.linked = Some(cam.clone());
                self.render_apply_scene_camera(&cam);
                self.render_apply(patch(RenderOutputPatch { camera_near: Some(0.1), camera_far: Some(200.0), ..Default::default() }));
                let g = &mut self.render_ui.prefs.guides;
                g.thirds = true;
                g.action_safe = true;
                g.title_safe = true;
            }
            "render-saida" => {
                self.set_tab(RenderTab::Output);
                self.render_apply(patch(RenderOutputPatch { width: Some(2480), height: Some(3508), dpi: Some(300), supersample: Some(2), format: Some(OutputFormat::Png16), ..Default::default() }));
                self.render_ui.aspect_ref = 2480.0 / 3508.0;
                self.script_aim_camera();
            }
            "render-passes" => {
                self.set_tab(RenderTab::Passes);
                self.script_aim_camera();
                // foco no personagem (o ator mais próximo da câmera); fundo desfocado
                self.scene_ui.selection.set_one(anigo_core::ids::NodeId::canonical_character());
                let focus = self.render_focus_distance().unwrap_or(8.0);
                self.scene_ui.selection.clear();
                let mut dof = self.core.project().render.output.dof;
                dof.enabled = true;
                dof.focus_distance = focus;
                dof.f_number = 1.0;
                dof.focal_mm = 200.0;
                dof.max_radius_px = 32.0;
                self.render_apply(patch(RenderOutputPatch { dof: Some(dof), aov: Some(AovSelection { outline: true, depth: true, mask: true, object_id: false, normal: false }), ..Default::default() }));
            }
            "render-transparente" => {
                self.set_tab(RenderTab::Output);
                self.render_apply(patch(RenderOutputPatch { transparent: Some(true), supersample: Some(2), ..Default::default() }));
                self.script_aim_camera();
            }
            "render-fila" => {
                self.set_tab(RenderTab::Output);
                let dir = std::env::var_os("ANIGO_RENDER_DIR")
                    .map(std::path::PathBuf::from)
                    .unwrap_or_else(|| std::env::temp_dir().join(format!("anigo-render-scenario-{}", std::process::id())));
                run.temp_dir = Some(dir.clone());
                self.render_ui.prefs.dir = dir.display().to_string();
                self.render_apply(patch(RenderOutputPatch {
                    width: Some(1280),
                    height: Some(720),
                    supersample: Some(2),
                    transparent: Some(true),
                    aov: Some(AovSelection { outline: true, depth: true, mask: false, object_id: true, normal: false }),
                    ..Default::default()
                }));
                self.render_ui.aspect_ref = 1280.0 / 720.0;
                self.script_aim_camera();
                self.render_enqueue(ctx);
            }
            _ => self.set_tab(RenderTab::Camera),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scenario_names_are_recognised_and_unique() {
        let mut seen = std::collections::BTreeSet::new();
        for n in RENDER_SCENARIOS {
            assert!(is_render_scenario(n) && seen.insert(*n));
        }
        assert!(!is_render_scenario("cenario-montado"));
    }
}
