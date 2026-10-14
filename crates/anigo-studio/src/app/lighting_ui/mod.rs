//! Workspace Iluminação nativo: controla a luz principal (sol), o ambiente hemisférico, a tinta e a sombra
//! projetada do projeto, com preview ao vivo sobre o personagem e o cenário.
//!
//! Toda mudança persistente é um `Command` do núcleo (`StudioApp::core`, a MESMA `CoreSession` do Personagem e
//! do Cenário): `SetLight` para a luz, `SetMaterialParams` para o terminador e o rim do personagem. Arrastar o
//! disco do sol (ou um slider) usa `CoreSession::preview` (ao vivo, fora do histórico) e vira UM passo de
//! desfazer ao soltar. A viewport reaproveita o renderer do Cenário (`scene_render`): é o mesmo cel canônico.
//!
//! * `viewport` — navegação, gizmo de direção (esfera + disco do sol), render e overlays;
//! * `panels` — rigs, sol, sombras, ambiente, rim, luzes extras (desabilitadas), cena de prova;
//! * `script` — cenários de captura (`--scenario iluminacao-*`).
//!
//! A matemática (az/el, temperatura, câmera-relativa, escolha sobre a esfera, rigs) mora em `crate::lighting`.

mod panels;
mod script;
mod viewport;

use super::StudioApp;
use crate::lighting::{self, ArcballHemisphere};
use crate::viewport::OrbitCamera;
use anigo_core::command::{Command, MaterialPatch};
use anigo_core::ids::LightId;
use anigo_core::scene::{LightShadow, StylizedLight, StylizedMaterial};
use glam::{Vec2, Vec3};

pub(crate) use script::is_light_scenario;
#[allow(unused_imports)]
pub use script::LIGHT_SCENARIOS;

/// Seção do painel destacada pela ferramenta do shelf.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Section {
    Rigs,
    Sun,
    Shadows,
    Ambient,
    Extra,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum DragKind {
    /// LMB sobre o disco do sol: escolhe a direção sobre a esfera.
    Disk,
    /// Ctrl+LMB em qualquer lugar: gira a luz pelo deslocamento do mouse.
    Relative,
}

/// Arrasto de luz em andamento.
pub(crate) struct LightDrag {
    pub kind: DragKind,
    pub hemi: ArcballHemisphere,
}

/// Edição ao vivo em andamento (uma mudança que vira UM passo de desfazer ao soltar).
pub(crate) struct Gesture {
    pub cmd: Command,
    /// Luz e material do personagem antes do gesto: se o resultado é igual, nada entra no histórico.
    pub base_light: StylizedLight,
    pub base_material: Option<StylizedMaterial>,
}

pub(crate) struct LightingUi {
    // ---- câmera e visão
    pub camera: OrbitCamera,
    /// Multi-vista do viewport; a câmera ativa é `camera`.
    pub mview: crate::app::multiview::MultiView,
    pub framed: bool,
    pub show_gizmo: bool,
    /// `light_enter` já rodou (entrada por `--workspace iluminacao` não passa por `switch_workspace`).
    pub entered: bool,
    // ---- luz presa à câmera (preview)
    pub follow_camera: bool,
    /// Azimute/elevação RELATIVOS à câmera (graus) enquanto `follow_camera`.
    pub rel_az: f32,
    pub rel_el: f32,
    /// Temperatura (K) do slider; a cor é a verdade, o K é a última escolha do usuário.
    pub temp_k: f32,
    // ---- interação
    pub drag: Option<LightDrag>,
    pub gesture: Option<Gesture>,
    pub hover_disk: bool,
    pub view_size: Vec2,
    /// Seção a abrir/rolar no próximo quadro (clique na ferramenta do shelf).
    pub focus: Option<Section>,
    pub last_rig: Option<&'static str>,
    // ---- estado
    pub message: String,
    pub message_seen: (String, Option<std::time::Instant>),
    // ---- cenários de captura / smoke
    pub script: Option<script::Script>,
    pub trace: Option<String>,
    /// Posição de tela (pixels físicos) do disco do sol e dos botões de rig no último quadro (smoke xdotool).
    pub disk_px: Option<Vec2>,
    pub rig_px: Vec<(&'static str, Vec2)>,
}

impl Default for LightingUi {
    fn default() -> Self {
        Self {
            camera: OrbitCamera { target: Vec3::new(0.0, 0.95, 0.0), yaw: 0.35, pitch: 0.14, distance: 5.6, far: 600.0, ..OrbitCamera::default() },
            mview: Default::default(),
            framed: false,
            show_gizmo: true,
            entered: false,
            follow_camera: false,
            rel_az: lighting::STUDIO_REL.0,
            rel_el: lighting::STUDIO_REL.1,
            temp_k: 5600.0,
            drag: None,
            gesture: None,
            hover_disk: false,
            view_size: Vec2::new(900.0, 600.0),
            focus: None,
            last_rig: None,
            message: String::new(),
            message_seen: (String::new(), None),
            script: None,
            trace: None,
            disk_px: None,
            rig_px: Vec::new(),
        }
    }
}

/// Valores que o painel edita, lidos do projeto (que, durante um arrasto, já reflete o preview).
#[derive(Clone)]
pub(crate) struct LightView {
    pub id: Option<LightId>,
    pub light: StylizedLight,
    pub material: Option<StylizedMaterial>,
}

/// Campos de `SetLight` com padrão `None` (construção de patches sem repetir 10 campos).
#[derive(Default, Clone)]
pub(crate) struct LightPatch {
    pub direction: Option<[f32; 3]>,
    pub color: Option<[f32; 3]>,
    pub intensity: Option<f32>,
    pub shadow_color: Option<[f32; 3]>,
    pub ambient_intensity: Option<f32>,
    pub shadow_saturation: Option<f32>,
    pub ambient_sky: Option<[f32; 3]>,
    pub ambient_ground: Option<[f32; 3]>,
    pub shadow: Option<LightShadow>,
}

impl LightPatch {
    pub fn into_command(self, id: Option<LightId>) -> Command {
        Command::SetLight {
            light_id: id,
            direction: self.direction,
            color: self.color,
            intensity: self.intensity,
            shadow_color: self.shadow_color,
            ambient_intensity: self.ambient_intensity,
            shadow_saturation: self.shadow_saturation,
            ambient_sky: self.ambient_sky,
            ambient_ground: self.ambient_ground,
            shadow: self.shadow,
        }
    }
}

/// Descrição de um comando do núcleo no idioma da UI (histórico/status do workspace).
pub(crate) fn describe_light(desc: &str) -> String {
    use crate::i18n::tr;
    if let Some(rest) = desc.strip_prefix("Undo ") {
        return format!("{}: {}", tr("char.undone"), describe_light(rest));
    }
    if let Some(rest) = desc.strip_prefix("Redo ") {
        return format!("{}: {}", tr("char.redone"), describe_light(rest));
    }
    let key = match desc {
        "Lighting" => "light.cmd.light",
        "Material" => "light.cmd.material",
        d if d.starts_with("Batch") => "light.cmd.rig",
        _ => return super::scene_ui::describe_scene(desc),
    };
    tr(key).to_string()
}

/// Luz e material do personagem de um projeto (com preview aplicado, se houver).
pub(crate) fn read_view(core: &anigo_session::CoreSession) -> LightView {
    let p = core.project();
    let slot = p.scene.lights.first();
    LightView {
        id: slot.map(|s| s.light_id.clone()),
        light: slot.map(|s| s.light.clone()).unwrap_or_default(),
        material: p.character_material().map(|m| m.material.clone()),
    }
}

/// Resultado de fechar uma edição ao vivo.
#[derive(Debug)]
pub(crate) enum GestureEnd {
    /// Nada estava aberto.
    Idle,
    /// O gesto voltou ao valor inicial: nada entrou no histórico.
    Cancelled,
    /// UM passo de desfazer com este comando final.
    Committed(anigo_core::command::CommandOutcome),
    /// O núcleo recusou o comando final (mensagem).
    Refused(String),
}

/// Abre/atualiza uma edição ao vivo: aplica `cmd` em preview (fora do histórico) e guarda o comando final.
pub(crate) fn gesture_edit(core: &mut anigo_session::CoreSession, slot: &mut Option<Gesture>, cmd: Command) {
    if slot.is_none() {
        let v = read_view(core);
        *slot = Some(Gesture { cmd: cmd.clone(), base_light: v.light, base_material: v.material });
    }
    let _ = core.preview(cmd.clone());
    if let Some(g) = slot.as_mut() {
        g.cmd = cmd;
    }
}

/// Luzes iguais a menos de 1e-3 (0,06°; voltar o slider ao ponto de partida passa por ida e volta de graus↔direção).
pub(crate) fn lights_close(a: &StylizedLight, b: &StylizedLight) -> bool {
    let close3 = |x: [f32; 3], y: [f32; 3]| x.iter().zip(&y).all(|(p, q)| (p - q).abs() < 1e-3);
    let close = |x: f32, y: f32| (x - y).abs() < 1e-3;
    close3(a.direction, b.direction)
        && close3(a.color, b.color)
        && close3(a.shadow_color, b.shadow_color)
        && close3(a.ambient_sky, b.ambient_sky)
        && close3(a.ambient_ground, b.ambient_ground)
        && close(a.intensity, b.intensity)
        && close(a.ambient_intensity, b.ambient_intensity)
        && close(a.shadow_saturation, b.shadow_saturation)
        && a.shadow == b.shadow
}

/// Fecha a edição ao vivo: UM passo de desfazer com o último comando, ou nenhum se luz e material voltaram
/// ao valor de partida (soltar o slider onde começou não suja o histórico).
pub(crate) fn gesture_finish(core: &mut anigo_session::CoreSession, slot: &mut Option<Gesture>) -> GestureEnd {
    let Some(g) = slot.take() else { return GestureEnd::Idle };
    let now = read_view(core);
    if lights_close(&now.light, &g.base_light) && now.material == g.base_material {
        core.cancel_preview();
        return GestureEnd::Cancelled;
    }
    match core.commit_preview(g.cmd) {
        Ok(o) => GestureEnd::Committed(o),
        Err(e) => GestureEnd::Refused(e),
    }
}

impl StudioApp {
    pub(crate) fn light_say(&mut self, msg: impl Into<String>) {
        self.light_ui.message = msg.into();
    }

    /// Luz e material do personagem do projeto (com preview aplicado, se houver).
    pub(crate) fn light_view(&self) -> LightView {
        read_view(&self.core)
    }

    /// Aplica um comando e reporta no status. `true` se aceito.
    pub(crate) fn light_apply(&mut self, cmd: Command) -> bool {
        match self.core.apply(cmd) {
            Ok(o) => {
                self.light_ui.message = describe_light(&o.description);
                true
            }
            Err(e) => {
                if !e.contains("no effect") && !e.contains("already") && !e.contains("empty") {
                    self.light_ui.message = format!("{}: {e}", crate::i18n::tr("char.error"));
                }
                false
            }
        }
    }

    /// Edição ao vivo (arrasto de slider/gizmo): preview agora, UM passo de desfazer ao soltar.
    pub(crate) fn light_edit(&mut self, cmd: Command) {
        gesture_edit(&mut self.core, &mut self.light_ui.gesture, cmd);
    }

    pub(crate) fn light_edit_patch(&mut self, patch: LightPatch) {
        let id = self.light_view().id;
        self.light_edit(patch.into_command(id));
    }

    pub(crate) fn light_edit_material(&mut self, patch: MaterialPatch) {
        self.light_edit(Command::SetMaterialParams { material_id: None, patch });
    }

    /// Termina a edição ao vivo quando o ponteiro é solto: UM passo de desfazer (ou nenhum, se voltou ao valor inicial).
    pub(crate) fn light_finish_gesture(&mut self, ctx: &egui::Context) {
        if self.light_ui.gesture.is_none() || ctx.input(|i| i.pointer.any_down()) {
            return;
        }
        match gesture_finish(&mut self.core, &mut self.light_ui.gesture) {
            GestureEnd::Committed(o) => self.light_ui.message = describe_light(&o.description),
            GestureEnd::Refused(e) => {
                if !e.contains("no effect") && !e.contains("already") {
                    self.light_ui.message = e;
                }
            }
            GestureEnd::Idle | GestureEnd::Cancelled => {}
        }
    }

    /// Direção efetiva do sol: a do projeto, ou a relativa à câmera no modo "luz presa à câmera".
    pub(crate) fn light_effective_dir(&self) -> Vec3 {
        if self.light_ui.follow_camera {
            lighting::camera_relative_dir(&self.light_ui.camera, self.light_ui.rel_az, self.light_ui.rel_el)
        } else {
            Vec3::from_array(self.light_view().light.direction).normalize_or_zero()
        }
    }

    /// Ctrl+Z / Ctrl+Y (e Ctrl+Shift+Z) roteados para a sessão do núcleo; fora de campos de texto.
    pub(crate) fn light_shortcuts(&mut self, ctx: &egui::Context) {
        self.character_shortcuts(ctx);
    }

    /// Entra no workspace: histórico profundo (como o Cenário) e câmera enquadrada no personagem.
    pub(crate) fn light_enter(&mut self) {
        self.light_ui.entered = true;
        self.core.set_history_limit(400);
        self.light_ui.framed = false;
    }

    /// Clique numa ferramenta do shelf: destaca a seção correspondente.
    pub(crate) fn on_light_tool_selected(&mut self, id: &str) {
        self.light_ui.focus = match id {
            "sun" => Some(Section::Sun),
            "shadows" => Some(Section::Shadows),
            "ambient" => Some(Section::Ambient),
            "rigs" => Some(Section::Rigs),
            _ => None,
        };
    }

    /// Aplica um rig do catálogo (UM passo de desfazer) e desliga o modo de luz presa à câmera.
    pub(crate) fn light_apply_rig(&mut self, id: &str) {
        let Some(rig) = lighting::rigs::rig_by_id(id) else { return };
        self.light_ui.follow_camera = false;
        self.scene_ui.light_dir_override = None;
        let cmd = lighting::rigs::rig_command(self.core.project(), rig);
        if self.light_apply(cmd) {
            self.light_ui.last_rig = Some(rig.id);
            self.light_ui.temp_k = rig.kelvin;
            self.light_say(format!("{}: {}", crate::i18n::tr("light.rig_applied"), crate::i18n::tr(rig.label_key)));
        }
    }

    /// Grava a direção relativa à câmera (luz presa) como a direção do projeto — UM passo de desfazer.
    pub(crate) fn light_bake_follow(&mut self) {
        let dir = lighting::camera_relative_dir(&self.light_ui.camera, self.light_ui.rel_az, self.light_ui.rel_el);
        if dir == Vec3::ZERO {
            return;
        }
        let id = self.light_view().id;
        let cmd = LightPatch { direction: Some(dir.to_array()), ..Default::default() }.into_command(id);
        if self.light_apply(cmd) {
            self.light_ui.follow_camera = false;
            self.scene_ui.light_dir_override = None;
        }
    }

    /// Liga/desliga a luz presa à câmera, partindo da direção atual do sol (a luz não "pula").
    pub(crate) fn light_set_follow(&mut self, on: bool) {
        if on == self.light_ui.follow_camera {
            return;
        }
        if on {
            let dir = Vec3::from_array(self.light_view().light.direction);
            let (az, el) = lighting::camera_relative_angles(&self.light_ui.camera, dir, self.light_ui.rel_az);
            self.light_ui.rel_az = az;
            self.light_ui.rel_el = el;
        } else {
            self.scene_ui.light_dir_override = None;
        }
        self.light_ui.follow_camera = on;
    }

    /// Centro do gizmo: o centro do personagem (ou um ponto padrão antes da geometria carregar).
    pub(crate) fn light_pivot(&self) -> Vec3 {
        self.scene_ui
            .runtime
            .bounds_of(&anigo_core::ids::NodeId::canonical_character())
            .map(|b| b.center())
            .unwrap_or(Vec3::new(0.0, 0.95, 0.0))
    }

    /// Enquadra o personagem na câmera do workspace.
    pub(crate) fn light_frame_character(&mut self) {
        if let Some(b) = self.scene_ui.runtime.bounds_of(&anigo_core::ids::NodeId::canonical_character()) {
            let c = &mut self.light_ui.camera;
            crate::scene::camview::frame_box(c, b.min, b.max);
            c.distance = (c.distance * 0.95).clamp(1.8, 30.0);
            self.light_ui.framed = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_descriptions_are_localised() {
        for raw in ["Lighting", "Material", "Batch 2", "Undo Lighting", "Redo Batch 3"] {
            let out = describe_light(raw);
            assert_ne!(out, raw, "sem tradução: {raw}");
        }
        assert_eq!(describe_light("Lighting"), crate::i18n::tr("light.cmd.light"));
    }

    fn sun_cmd(az: f32, el: f32) -> Command {
        LightPatch { direction: Some(lighting::dir_from_az_el(az, el).to_array()), ..Default::default() }.into_command(None)
    }

    #[test]
    fn a_drag_of_many_previews_is_exactly_one_undo_step() {
        let mut core = anigo_session::CoreSession::new();
        let before = core.document().unwrap();
        let mut slot = None;
        for step in 0..40 {
            gesture_edit(&mut core, &mut slot, sun_cmd(-80.0 + step as f32 * 4.0, 20.0 + step as f32));
            assert!(core.is_previewing());
            assert_eq!(core.history().undo_depth(), 0, "preview não entra no histórico");
        }
        let live = read_view(&core).light.direction;
        let end = gesture_finish(&mut core, &mut slot);
        assert!(matches!(end, GestureEnd::Committed(_)), "{end:?}");
        assert_eq!(core.history().undo_depth(), 1, "UM passo, não 40");
        assert_eq!(read_view(&core).light.direction, live, "o commit grava o último valor do arrasto");
        core.undo().unwrap();
        assert_eq!(core.document().unwrap(), before, "undo restaura o documento exato");
        core.redo().unwrap();
        assert_eq!(read_view(&core).light.direction, live);
    }

    #[test]
    fn returning_to_the_starting_value_leaves_the_history_clean() {
        let mut core = anigo_session::CoreSession::new();
        let start = Vec3::from_array(read_view(&core).light.direction);
        let (az, el) = lighting::az_el_from_dir(start, 0.0);
        let mut slot = None;
        gesture_edit(&mut core, &mut slot, sun_cmd(az + 30.0, el));
        gesture_edit(&mut core, &mut slot, sun_cmd(az, el));
        let end = gesture_finish(&mut core, &mut slot);
        assert!(matches!(end, GestureEnd::Cancelled), "{end:?} start={start:?} now={:?}", read_view(&core).light.direction);
        assert_eq!(core.history().undo_depth(), 0);
        assert!(!core.is_previewing());
        assert!(matches!(gesture_finish(&mut core, &mut slot), GestureEnd::Idle));
    }

    #[test]
    fn material_and_shadow_edits_are_gestures_too() {
        let mut core = anigo_session::CoreSession::new();
        let mut slot = None;
        for v in [0.5, 1.5, 2.4] {
            gesture_edit(&mut core, &mut slot, Command::SetMaterialParams { material_id: None, patch: MaterialPatch { rim_intensity: Some(v), ..Default::default() } });
        }
        assert!(matches!(gesture_finish(&mut core, &mut slot), GestureEnd::Committed(_)));
        assert_eq!(read_view(&core).material.unwrap().rim_intensity, 2.4);
        let sh = LightShadow { enabled: true, strength: 0.6, ..Default::default() };
        gesture_edit(&mut core, &mut slot, LightPatch { shadow: Some(sh), ..Default::default() }.into_command(None));
        assert!(matches!(gesture_finish(&mut core, &mut slot), GestureEnd::Committed(_)));
        assert_eq!(read_view(&core).light.shadow, sh);
        assert_eq!(core.history().undo_depth(), 2);
        core.undo().unwrap();
        assert!(!read_view(&core).light.shadow.enabled, "undo desliga a sombra");
        core.undo().unwrap();
        assert_eq!(read_view(&core).material.unwrap().rim_intensity, StylizedMaterial::default().rim_intensity);
    }

    #[test]
    fn an_invalid_final_command_is_refused_and_restores_the_base() {
        let mut core = anigo_session::CoreSession::new();
        let mut slot = None;
        let bad = LightShadow { strength: 3.0, ..Default::default() };
        gesture_edit(&mut core, &mut slot, LightPatch { shadow: Some(bad), ..Default::default() }.into_command(None));
        // o preview inválido nem chegou a mudar o projeto: nada a comitar
        assert!(matches!(gesture_finish(&mut core, &mut slot), GestureEnd::Cancelled));
        assert_eq!(core.history().undo_depth(), 0);
        assert!(!read_view(&core).light.shadow.enabled);
    }

    #[test]
    fn light_patch_builds_a_single_set_light() {
        let cmd = LightPatch { intensity: Some(0.4), ..Default::default() }.into_command(None);
        match cmd {
            Command::SetLight { intensity, direction, shadow, .. } => {
                assert_eq!(intensity, Some(0.4));
                assert!(direction.is_none() && shadow.is_none());
            }
            other => panic!("esperava SetLight, veio {other:?}"),
        }
    }
}
