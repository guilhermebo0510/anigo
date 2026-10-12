//! Cenários de captura do Posing (`--scenario posing-*`): montam uma pose DE VERDADE (comandos do núcleo,
//! presets e IK reais) e levam a UI a um estado conhecido. O screenshot só sai quando a cena assentou.

use super::viewport::PoseTool;
use super::{HandSide, StudioApp};
use crate::library_ui::LibraryWorkspace;
use anigo_core::pose::{bone, presets, LimitProfile, PoseOp, PoseState};
use glam::{Quat, Vec3};

#[allow(dead_code)]
pub const POSING_SCENARIOS: &[&str] = &["posing-vazio", "posing-fk", "posing-limites", "posing-ik", "posing-presets", "posing-mao", "posing-gaze", "posing-biblioteca"];

pub(crate) fn is_posing_scenario(name: &str) -> bool {
    name.starts_with("posing-")
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

/// Biblioteca temporária (vazia) do cenário `posing-biblioteca`.
pub(crate) fn temp_library() -> Result<(LibraryWorkspace, std::path::PathBuf), String> {
    let dir = std::env::temp_dir().join(format!("anigo-posing-lib-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let lib = anigo_assets::Library::open(&dir).map_err(|e| e.to_string())?;
    Ok((LibraryWorkspace::with_library(lib), dir))
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

impl StudioApp {
    pub(crate) fn posing_start_script(&mut self, name: &str) {
        self.ws = crate::workspace::WorkspaceId::Posing;
        self.pose_ui.script = Some(Script { name: name.to_string(), stage: 0, stage_frame: 0, frame: 0, done: false });
    }

    fn set_posing_tool(&mut self, t: PoseTool) {
        let idx = match t {
            PoseTool::Rig => 0,
            PoseTool::Ik => 1,
            PoseTool::Presets => 2,
        };
        self.tool_idx.insert(crate::workspace::WorkspaceId::Posing, idx);
    }

    /// Máquina de estados dos cenários (chamada a cada quadro do workspace).
    pub(crate) fn posing_script_tick(&mut self) {
        let Some(mut run) = self.pose_ui.script.take() else { return };
        if run.done {
            self.pose_ui.script = Some(run);
            return;
        }
        run.frame += 1;
        let waited = run.frame.saturating_sub(run.stage_frame);
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
                let name = run.name.clone();
                self.build_posing_scene(&name);
                next = true;
            }
            2 => {
                if waited > 14 {
                    if run.name == "posing-biblioteca" {
                        self.pose_refresh_library();
                    }
                    next = true;
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
        if run.frame > 1500 {
            run.done = true;
        }
        self.pose_ui.script = Some(run);
    }

    fn scn_commit(&mut self, p: PoseState, op: PoseOp) {
        self.pose_commit_to(p, op);
    }

    fn build_posing_scene(&mut self, name: &str) {
        let rig = self.pose_rig();
        self.pose_ui.show_skeleton = true;
        self.pose_ui.show_limits = true;
        self.pose_ui.profile = LimitProfile::Human;
        match name {
            "posing-vazio" => {
                self.set_posing_tool(PoseTool::Rig);
            }
            "posing-fk" => {
                self.set_posing_tool(PoseTool::Rig);
                let mut p = presets::body("standing", &rig).unwrap_or_default();
                p.set_rotation(bone::L_SHOULDER, rx(-35.0) * rz(62.0));
                p.set_rotation(bone::L_ELBOW, rx(-70.0));
                p.set_rotation(bone::R_SHOULDER, rz(-14.0));
                p.set_rotation(bone::HEAD, ry(18.0) * rx(-6.0));
                p.set_rotation(bone::SPINE, ry(-8.0));
                self.scn_commit(p, PoseOp::Preset);
                self.pose_ui.select_one(bone::L_ELBOW);
                self.pose_ui.camera.yaw = 0.62;
            }
            "posing-limites" => {
                self.set_posing_tool(PoseTool::Rig);
                let mut p = presets::body("standing", &rig).unwrap_or_default();
                // cotovelo dobrado além do limite (170° > 150°): o cone fica vermelho
                p.set_rotation(bone::L_SHOULDER, rx(-50.0) * rz(40.0));
                p.set_rotation(bone::L_ELBOW, rx(-168.0));
                p.set_rotation(bone::R_KNEE, rx(20.0));
                self.scn_commit(p, PoseOp::Edit);
                self.pose_ui.select_one(bone::L_ELBOW);
                self.pose_ui.camera.target = Vec3::new(0.25, 1.15, 0.2);
                self.pose_ui.camera.distance = 1.9;
                self.pose_ui.camera.yaw = 1.35;
                self.pose_ui.camera.pitch = 0.08;
            }
            "posing-ik" => {
                self.set_posing_tool(PoseTool::Ik);
                self.pose_ui.limb_ik = [true; 4];
                self.pose_ui.foot_lock = true;
                self.pose_ui.show_balance = true;
                self.pose_ui.select_one(bone::HIPS);
                // quadril abaixado com os pés travados (IK de perna real)
                let start = self.pose_now();
                let mv = crate::posing::edit::HipsMove { delta: Vec3::new(0.04, -0.22, 0.03), whole_body: false, foot_lock: true, auto_ground: false };
                let mut t = crate::posing::edit::move_hips(&rig, &start, mv, &self.pose_ui.edit_opts());
                // braço esquerdo alcançando à frente por IK
                let j = rig.joint_positions(&t);
                let target = j[bone::L_WRIST] + Vec3::new(0.05, 0.38, 0.32);
                t = crate::posing::edit::drag_effector(&rig, &t, anigo_core::pose::Limb::LeftArm, target, None, &self.pose_ui.edit_opts());
                self.scn_commit(t, PoseOp::Ik);
                self.pose_ui.camera.yaw = 0.75;
            }
            "posing-presets" => {
                self.set_posing_tool(PoseTool::Presets);
                self.pose_apply_body_preset("action");
                self.pose_ui.camera.yaw = -0.5;
            }
            "posing-mao" => {
                // braços esticados para a frente: os dedos curvam para cima e aparecem de perfil
                self.set_posing_tool(PoseTool::Presets);
                self.pose_apply_body_preset("standing");
                let mut p = self.pose_now();
                p.set_rotation(bone::L_SHOULDER, rx(-90.0));
                p.set_rotation(bone::R_SHOULDER, rx(-90.0));
                p.set_rotation(bone::L_ELBOW, rx(0.0));
                p.hands.left = presets::hand("point").unwrap_or_default();
                p.hands.right = presets::hand("fist").unwrap_or_default();
                self.scn_commit(p, PoseOp::Hand);
                self.pose_ui.hand_side = HandSide::Left;
                self.pose_ui.camera.target = Vec3::new(0.0, 1.4, 0.6);
                self.pose_ui.camera.distance = 1.35;
                self.pose_ui.camera.yaw = 1.15;
                self.pose_ui.camera.pitch = 0.18;
                self.pose_ui.show_skeleton = false;
            }
            "posing-gaze" => {
                self.set_posing_tool(PoseTool::Ik);
                self.pose_apply_body_preset("standing");
                let eye = anigo_ik::pose_ik::eye_center(&rig, &self.pose_now());
                self.pose_ui.gaze.enabled = true;
                self.pose_ui.gaze.target = eye + Vec3::new(0.55, 0.22, 0.9);
                let t = anigo_ik::pose_ik::look_at_head(&rig, &self.pose_now(), self.pose_ui.gaze.target, 1.0, 0.35, LimitProfile::Human).pose;
                self.scn_commit(t, PoseOp::Gaze);
                self.pose_ui.select_one(bone::HEAD);
                self.pose_ui.camera.yaw = 0.35;
            }
            "posing-biblioteca" => {
                self.set_posing_tool(PoseTool::Presets);
                for (key, label) in [("sitting", "Sentada"), ("walking", "Caminhada"), ("t", "T-pose")] {
                    self.pose_apply_body_preset(key);
                    self.pose_ui.lib_name = label.to_string();
                    self.pose_ui.lib_only_selected = false;
                    self.pose_save_to_library();
                }
                // só o braço esquerdo da pose de ação, como ativo parcial
                self.pose_apply_body_preset("action");
                self.pose_ui.selection = [bone::L_SHOULDER, bone::L_ELBOW, bone::L_WRIST].into_iter().collect();
                self.pose_ui.active = Some(bone::L_ELBOW);
                self.pose_ui.lib_name = "Soco esquerdo".into();
                self.pose_ui.lib_only_selected = true;
                self.pose_save_to_library();
                self.pose_ui.lib_only_selected = false;
                self.pose_ui.clear_selection();
                self.pose_apply_body_preset("standing");
                self.pose_refresh_library();
                // aplica "Caminhada" espelhada na pose em pé (clique de verdade no card, caminho do botão)
                if let Some(i) = self.pose_ui.poses.iter().position(|c| c.name == "Caminhada") {
                    self.pose_apply_asset(i, true);
                }
                self.pose_ui.camera.yaw = 0.7;
            }
            _ => {}
        }
        // `ANIGO_POSING_CAM="tx,ty,tz,distância,yaw,pitch"` sobrescreve a câmera do cenário (inspeção de detalhes)
        if let Some(v) = std::env::var("ANIGO_POSING_CAM").ok().map(|s| s.split(',').filter_map(|x| x.trim().parse::<f32>().ok()).collect::<Vec<_>>()).filter(|v| v.len() == 6) {
            self.pose_ui.camera.target = Vec3::new(v[0], v[1], v[2]);
            self.pose_ui.camera.distance = v[3];
            self.pose_ui.camera.yaw = v[4];
            self.pose_ui.camera.pitch = v[5];
        }
        self.pose_ui.framed = true;
    }
}
