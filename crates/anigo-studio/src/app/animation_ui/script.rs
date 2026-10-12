//! Cenários de captura da Animação (`--scenario animacao-*`): montam um clip DE VERDADE pelo mesmo caminho do usuário
//! (editar a pose com o auto-key ligado em quadros diferentes) e levam a UI a um estado conhecido. O screenshot só sai
//! quando a cena assentou; `animacao-sequencia` faz um render real da sequência numa pasta temporária.

use super::{Editor, InterpPick, StudioApp};
use crate::app::render_ui::JobStatus;
use anigo_core::command::Command;
use anigo_core::pose::{bone, presets, PoseOp, PoseState};
use anigo_core::RenderOutputPatch;
use glam::{Quat, Vec3};

#[allow(dead_code)]
pub const ANIMATION_SCENARIOS: &[&str] = &["animacao-vazio", "animacao-timeline", "animacao-curvas", "animacao-passo", "animacao-bezier", "animacao-onion", "animacao-fisica", "animacao-clips", "animacao-sequencia"];

pub(crate) fn is_animation_scenario(name: &str) -> bool {
    name.starts_with("animacao-")
}

pub(crate) struct Script {
    pub name: String,
    stage: u32,
    stage_frame: u64,
    frame: u64,
    done: bool,
    /// Pasta temporária do render do cenário `animacao-sequencia` (apagada ao sair).
    pub temp_dir: Option<std::path::PathBuf>,
}

impl Script {
    pub fn is_done(&self) -> bool {
        self.done
    }
}

fn rx(d: f32) -> Quat {
    Quat::from_rotation_x(d.to_radians())
}
fn ry(d: f32) -> Quat {
    Quat::from_rotation_y(d.to_radians())
}
fn rz(d: f32) -> Quat {
    Quat::from_rotation_z(d.to_radians())
}

/// As três poses-chave do cenário ("acenar"): em pé, braço direito erguido e cotovelo dobrado, braço para o lado.
pub(crate) fn wave_poses(rig: &anigo_core::pose::PoseRig) -> [(i32, PoseState); 3] {
    let stand = presets::body("standing", rig).unwrap_or_default();
    let mut up = stand.clone();
    up.set_rotation(bone::R_SHOULDER, rz(-125.0) * rx(-10.0));
    up.set_rotation(bone::R_ELBOW, rx(-70.0));
    up.set_rotation(bone::HEAD, ry(-14.0));
    up.set_rotation(bone::SPINE, ry(8.0));
    up.hips_translation = [0.0, -0.05, 0.0];
    up.hands.right = presets::hand("open").unwrap_or_default();
    let mut side = stand.clone();
    side.set_rotation(bone::R_SHOULDER, rz(-70.0) * rx(15.0));
    side.set_rotation(bone::R_ELBOW, rx(-25.0));
    side.set_rotation(bone::L_SHOULDER, rz(35.0));
    side.set_rotation(bone::HEAD, ry(14.0));
    side.set_rotation(bone::SPINE, ry(-8.0));
    side.hips_translation = [0.0, 0.0, 0.0];
    [(0, stand), (12, up), (24, side)]
}

impl StudioApp {
    pub(crate) fn anim_start_script(&mut self, name: &str) {
        self.ws = crate::workspace::WorkspaceId::Animacao;
        self.anim_ui.script = Some(Script { name: name.to_string(), stage: 0, stage_frame: 0, frame: 0, done: false, temp_dir: None });
    }

    /// Apaga a pasta temporária do cenário (a menos que `ANIGO_SCENARIO_KEEP` esteja definida).
    pub(crate) fn anim_cleanup(&mut self) {
        if std::env::var_os("ANIGO_SCENARIO_KEEP").is_some() {
            return;
        }
        if let Some(d) = self.anim_ui.script.as_mut().and_then(|s| s.temp_dir.take()) {
            let _ = std::fs::remove_dir_all(d);
        }
    }

    fn set_editor(&mut self, e: Editor) {
        let idx = match e {
            Editor::Dope => 0,
            Editor::Curves => 1,
            Editor::Clips => 2,
        };
        self.tool_idx.insert(crate::workspace::WorkspaceId::Animacao, idx);
    }

    /// Edita a pose no quadro `frame` pela rota do usuário (auto-key ligado): um passo de histórico por quadro.
    fn scn_key_pose(&mut self, frame: i32, pose: PoseState) {
        self.anim_ui.transport.goto(frame as f32, true);
        self.anim_sync();
        self.pose_commit_to(pose, PoseOp::Edit);
    }

    /// Monta o clip "Acenar" (3 poses-chave, quadris/mão/raiz) com a interpolação pedida.
    fn scn_build_clip(&mut self, interp: InterpPick) {
        let rig = self.pose_rig();
        self.anim_ui.autokey = true;
        self.anim_ui.interp = interp;
        for (frame, pose) in wave_poses(&rig) {
            self.scn_key_pose(frame, pose);
        }
        // nomeia o clip
        if let Some(id) = self.anim_clip_id() {
            self.anim_apply(anigo_core::animation::AnimEdit::RenameClip { clip_id: id, name: "Acenar".to_string() });
        }
        self.anim_ui.autokey = false;
        self.anim_ui.interp = InterpPick::Ease;
    }

    /// Dá ao primeiro canal de osso do clip um Bézier de ease forte (para o cenário `animacao-bezier`/curvas).
    fn scn_strong_bezier(&mut self) {
        let (Some(id), Some(clip)) = (self.anim_clip_id(), self.anim_clip().cloned()) else { return };
        let keys: Vec<anigo_core::animation::KeyRef> = clip.channels.iter().flat_map(|c| c.keys.iter().map(|k| anigo_core::animation::KeyRef { target: c.target.clone(), frame: k.frame })).collect();
        self.anim_apply(anigo_core::animation::AnimEdit::SetInterpolation { clip_id: id, keys, interp: anigo_core::animation::Interp::Bezier { c: [0.9, 0.0, 0.1, 1.0] } });
    }

    /// Posição do playhead do cenário: `ANIGO_ANIM_FRAME` sobrescreve (inspeção em tempos diferentes).
    fn scn_goto(&mut self, default: i32) {
        let f = std::env::var("ANIGO_ANIM_FRAME").ok().and_then(|v| v.parse::<i32>().ok()).unwrap_or(default);
        self.anim_ui.transport.goto(f as f32, true);
        self.anim_sync();
    }

    fn build_animation_scene(&mut self, name: &str) {
        self.pose_ui.show_skeleton = true;
        self.pose_ui.show_limits = false;
        self.pose_ui.show_labels = false;
        self.anim_ui.side_tab = super::SideTab::Clip;
        match name {
            "animacao-vazio" => self.set_editor(Editor::Dope),
            "animacao-timeline" => {
                self.set_editor(Editor::Dope);
                self.scn_build_clip(InterpPick::Ease);
                self.scn_goto(6);
                self.anim_ui.selection = [(anigo_core::animation::ChannelTarget::bone("RightElbow"), 12), (anigo_core::animation::ChannelTarget::bone("RightShoulder"), 12)].into_iter().collect();
                self.pose_ui.camera.yaw = 0.4;
            }
            "animacao-curvas" => {
                self.set_editor(Editor::Curves);
                self.scn_build_clip(InterpPick::Ease);
                self.scn_strong_bezier();
                self.scn_goto(8);
                self.anim_ui.graph_visible = [anigo_core::animation::ChannelTarget::bone("RightShoulder"), anigo_core::animation::ChannelTarget::bone("RightElbow")].into_iter().collect();
                self.anim_ui.selection = [(anigo_core::animation::ChannelTarget::bone("RightShoulder"), 0)].into_iter().collect();
                self.pose_ui.camera.yaw = 0.4;
            }
            "animacao-passo" => {
                self.set_editor(Editor::Dope);
                self.scn_build_clip(InterpPick::Step);
                self.scn_goto(8);
                self.pose_ui.camera.yaw = 0.4;
            }
            "animacao-bezier" => {
                self.set_editor(Editor::Dope);
                self.scn_build_clip(InterpPick::Bezier);
                self.scn_strong_bezier();
                self.scn_goto(8);
                self.pose_ui.camera.yaw = 0.4;
            }
            "animacao-onion" => {
                self.set_editor(Editor::Dope);
                self.scn_build_clip(InterpPick::Linear);
                self.scn_goto(12);
                self.anim_ui.onion.enabled = true;
                self.anim_ui.onion.on_keys = true;
                self.anim_ui.onion.before = 1;
                self.anim_ui.onion.after = 1;
                self.pose_ui.camera.yaw = 0.4;
            }
            "animacao-fisica" => {
                self.set_editor(Editor::Dope);
                self.scn_build_clip(InterpPick::Ease);
                self.scn_goto(18);
                self.anim_ui.phys.enabled = true;
                self.anim_ui.side_tab = super::SideTab::Clip;
                self.anim_ui.force_section = Some("phys");
                self.pose_ui.camera.yaw = 0.9;
                self.pose_ui.show_skeleton = false;
            }
            "animacao-clips" => {
                self.set_editor(Editor::Clips);
                self.scn_build_clip(InterpPick::Ease);
                // salva na Biblioteca (rota do botão), duplica e cria um clip novo
                self.anim_ui.lib_name = "Acenar".into();
                self.anim_save_clip_to_library();
                if let Some(id) = self.anim_clip_id() {
                    self.anim_duplicate_clip(&id);
                }
                self.anim_new_clip();
                self.anim_refresh_library();
                if let Some(first) = self.core.project().animation.clips.keys().next().cloned() {
                    self.anim_select_clip(&first);
                }
                self.scn_goto(12);
            }
            "animacao-sequencia" => {
                self.set_editor(Editor::Dope);
                self.scn_build_clip(InterpPick::Ease);
                self.scn_goto(0);
                self.anim_ui.force_section = Some("seq");
                self.pose_ui.camera.yaw = 0.4;
            }
            _ => {}
        }
        // `ANIGO_POSING_CAM` também vale aqui (mesmo formato do Posing)
        if let Some(v) = std::env::var("ANIGO_POSING_CAM").ok().map(|s| s.split(',').filter_map(|x| x.trim().parse::<f32>().ok()).collect::<Vec<_>>()).filter(|v| v.len() == 6) {
            self.pose_ui.camera.target = Vec3::new(v[0], v[1], v[2]);
            self.pose_ui.camera.distance = v[3];
            self.pose_ui.camera.yaw = v[4];
            self.pose_ui.camera.pitch = v[5];
        }
        self.pose_ui.framed = true;
    }

    /// Ajusta a saída/câmera do render do cenário e enfileira a sequência (0..24 de 8 em 8 = 4 quadros).
    fn scn_queue_sequence(&mut self, ctx: &egui::Context, run: &mut Script) {
        let dir = std::env::var_os("ANIGO_RENDER_DIR").map(std::path::PathBuf::from).unwrap_or_else(|| std::env::temp_dir().join(format!("anigo-anim-scenario-{}", std::process::id())));
        run.temp_dir = Some(dir.clone());
        self.render_ui.prefs.dir = dir.display().to_string();
        self.render_ui.prefs.overwrite = true;
        let patch = RenderOutputPatch { width: Some(640), height: Some(360), supersample: Some(1), transparent: Some(false), ..Default::default() };
        self.core.apply(Command::SetRenderOutput { patch }).ok();
        self.core.apply(Command::SetCamera { eye: Some([1.2, 1.2, 4.6]), target: Some([0.0, 0.88, 0.0]), up: Some([0.0, 1.0, 0.0]), fov_degrees: Some(30.0), projection: None }).ok();
        self.anim_ui.seq_custom = true;
        self.anim_ui.seq_start = 0;
        self.anim_ui.seq_end = 24;
        self.anim_ui.seq_step = 8;
        self.anim_render_sequence(ctx);
    }

    /// Máquina de estados dos cenários (chamada a cada quadro do workspace).
    pub(crate) fn anim_script_tick(&mut self, ctx: &egui::Context) {
        let Some(mut run) = self.anim_ui.script.take() else { return };
        if run.done {
            self.anim_ui.script = Some(run);
            return;
        }
        run.frame += 1;
        let waited = run.frame.saturating_sub(run.stage_frame);
        let name = run.name.clone();
        let mut next = false;
        match run.stage {
            0 => {
                // espera o primeiro render (GPU/cena prontas e personagem enquadrado)
                if run.frame > 8 && self.pose_ui.framed {
                    self.core.set_history_limit(400);
                    next = true;
                }
                if run.frame > 120 {
                    next = true;
                }
            }
            1 => {
                self.build_animation_scene(&name);
                next = true;
            }
            2 => {
                if name == "animacao-sequencia" {
                    // espera a cena (malhas) carregar; então enfileira o render real
                    if waited > 4 && self.build_render_scene().is_ok() {
                        self.scn_queue_sequence(ctx, &mut run);
                        next = true;
                    }
                    if waited > 900 {
                        self.anim_say("sem cena para renderizar");
                        next = true;
                    }
                } else if waited > 14 {
                    next = true;
                }
            }
            3 => {
                if name == "animacao-sequencia" {
                    let finished = self.render_ui.jobs.last().is_some_and(|j| !matches!(j.status, JobStatus::Queued | JobStatus::Running));
                    if (finished && waited > 6) || waited > 2400 {
                        next = true;
                    }
                } else if waited > 8 {
                    run.done = true;
                }
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
        if run.frame > 3500 {
            run.done = true;
        }
        self.anim_ui.script = Some(run);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scenario_names_are_recognised_and_unique() {
        let mut seen = std::collections::BTreeSet::new();
        for n in ANIMATION_SCENARIOS {
            assert!(is_animation_scenario(n) && seen.insert(*n));
        }
        assert!(!is_animation_scenario("posing-fk"));
    }

    #[test]
    fn the_wave_poses_differ_and_start_in_the_standing_pose() {
        let rig = anigo_core::pose::PoseRig::default();
        let p = wave_poses(&rig);
        assert_eq!(p[0].0, 0);
        assert_ne!(p[0].1, p[1].1);
        assert_ne!(p[1].1, p[2].1);
    }
}
