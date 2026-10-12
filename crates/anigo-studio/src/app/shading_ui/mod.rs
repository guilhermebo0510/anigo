//! Workspace Shading nativo: editor de materiais toon/anime sobre o renderer cel canônico.
//!
//! * `sources` — de onde vêm os materiais (projeto/personagem, nós do Cenário, objetos da Modelagem), leitura e
//!   escrita por comandos do núcleo (`SetMaterialParams`, `SetNodeProps`) ou pelo histórico da Modelagem, com
//!   arrasto ao vivo (`preview`) que vira UM passo de desfazer ao soltar;
//! * `panels` — alvos, presets, parâmetros agrupados, editor de rampa, paleta, texturas;
//! * `viewport` — preview em shader ball + objeto + comparação A/B pelo `CelPreview` (renderer cel canônico);
//! * `script` — cenários de captura (`--scenario shading-*`).
//!
//! A lógica pura (tabela de parâmetros, paleta, layout da bola, presets) está em `crate::shading`.

mod panels;
mod ramp;
mod script;
mod sources;
mod viewport;

use super::StudioApp;
use crate::shading::ball::{LightRig, PreviewMode};
use crate::shading::palette::{Harmony, Palette, ShadeRecipe};
use crate::shading::params::Group;
use crate::shading::presets::PresetRow;
use crate::viewport::OrbitCamera;
use anigo_assets::AssetId;
use anigo_core::StylizedMaterial;
use std::collections::HashMap;
use std::sync::Arc;

pub(crate) use script::is_shading_scenario;

pub(crate) use crate::shading::edit::{Gesture, Target};

/// Aba de origem dos alvos (a mesma ordem dos workspaces que ela serve).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Source {
    #[default]
    Personagem,
    Cenario,
    Modelagem,
}

impl Source {
    pub const ALL: [Source; 3] = [Source::Personagem, Source::Cenario, Source::Modelagem];
    pub fn key(self) -> &'static str {
        match self {
            Source::Personagem => "shading.src.character",
            Source::Cenario => "shading.src.scene",
            Source::Modelagem => "shading.src.modeling",
        }
    }
}

/// Mapa de face SDF ativo no renderer.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) enum FaceMap {
    #[default]
    None,
    Procedural,
    Library(AssetId, String),
}

/// Aba da lista de presets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum PresetTab {
    #[default]
    BuiltIn,
    Library,
}

pub(crate) struct ShadingUi {
    // ---- alvo
    pub source: Source,
    pub target: Option<Target>,
    pub gesture: Option<Gesture>,
    pub clipboard: Option<StylizedMaterial>,
    // ---- preview
    pub mode: PreviewMode,
    pub compare: bool,
    pub snapshot_a: Option<StylizedMaterial>,
    pub camera: OrbitCamera,
    pub framed: bool,
    pub layout_key: u64,
    pub rig: LightRig,
    pub bg: u8,
    pub turntable: bool,
    // ---- painéis
    pub focus_group: Option<Group>,
    pub palette_open: bool,
    pub message: String,
    pub list_search: String,
    // ---- rampa
    pub ramp_sel: Option<usize>,
    // ---- paleta
    pub palette: Palette,
    pub palette_loaded: bool,
    pub palette_name: String,
    pub recipe: ShadeRecipe,
    pub link_shade: bool,
    pub harmony: Harmony,
    pub hue_turn: f32,
    pub hue_start: Option<StylizedMaterial>,
    pub palette_dir: std::path::PathBuf,
    // ---- presets
    pub preset_tab: PresetTab,
    pub preset_name: String,
    pub preset_rows: Vec<PresetRow>,
    pub preset_rows_for: Option<u64>,
    pub thumbs: HashMap<String, egui::TextureHandle>,
    pub thumb_failed: std::collections::HashSet<String>,
    pub cel_thumbs: Option<Arc<crate::library_ui::cel_thumb::CelThumbnailRenderer>>,
    // ---- texturas
    pub face_map: FaceMap,
    pub face_map_error: Option<String>,
    pub tex_search: String,
    pub tex_refs: std::collections::BTreeMap<String, (AssetId, String)>,
    // ---- cache de geometria do preview (chave → malha)
    pub subject_cache: HashMap<String, Arc<anigo_core::Mesh>>,
    // ---- cenários de captura
    pub script: Option<script::Script>,
    pub history_limit_set: bool,
    pub trace: Option<String>,
    /// Retângulos (rastreio do smoke) já impressos.
    pub rect_log: HashMap<String, String>,
}

impl Default for ShadingUi {
    fn default() -> Self {
        Self {
            source: Source::Personagem,
            target: None,
            gesture: None,
            clipboard: None,
            mode: PreviewMode::Both,
            compare: false,
            snapshot_a: None,
            camera: OrbitCamera { target: glam::Vec3::ZERO, yaw: 0.0, pitch: 0.08, distance: 4.0, fov_y: 30f32.to_radians(), ..OrbitCamera::default() },
            framed: false,
            layout_key: 0,
            rig: LightRig::default(),
            bg: 0,
            turntable: false,
            focus_group: None,
            palette_open: false,
            message: String::new(),
            list_search: String::new(),
            ramp_sel: None,
            palette: Palette::new("projeto"),
            palette_loaded: false,
            palette_name: "minha-paleta".into(),
            recipe: ShadeRecipe::default(),
            link_shade: false,
            harmony: Harmony::Complementary,
            hue_turn: 0.0,
            hue_start: None,
            palette_dir: crate::shading::palette::default_dir(),
            preset_tab: PresetTab::BuiltIn,
            preset_name: String::new(),
            preset_rows: Vec::new(),
            preset_rows_for: None,
            thumbs: HashMap::new(),
            thumb_failed: Default::default(),
            cel_thumbs: None,
            face_map: FaceMap::None,
            face_map_error: None,
            tex_search: String::new(),
            tex_refs: Default::default(),
            subject_cache: HashMap::new(),
            script: None,
            history_limit_set: false,
            trace: None,
            rect_log: HashMap::new(),
        }
    }
}

/// Fundos do preview (cor do clear do renderer).
pub(crate) const BACKGROUNDS: [[f32; 4]; 3] = [[0.17, 0.18, 0.22, 1.0], [0.50, 0.52, 0.56, 1.0], [0.93, 0.93, 0.94, 1.0]];

/// Descrição de um comando do núcleo no idioma da UI (status/histórico do Shading).
pub(crate) fn describe_shading(desc: &str) -> String {
    use crate::i18n::tr;
    if let Some(rest) = desc.strip_prefix("Undo ") {
        return format!("{}: {}", tr("char.undone"), describe_shading(rest));
    }
    if let Some(rest) = desc.strip_prefix("Redo ") {
        return format!("{}: {}", tr("char.redone"), describe_shading(rest));
    }
    if desc == "Material" || desc.starts_with("Properties ") {
        return tr("shading.cmd.material").to_string();
    }
    super::scene_ui::describe_scene(desc)
}

impl StudioApp {
    /// Imprime uma linha de rastreio quando algo relevante mudou (`ANIGO_TRACE_SHADING=1`): usada pelo smoke xdotool.
    pub(crate) fn shading_trace(&mut self, extra: &str) {
        if std::env::var_os("ANIGO_TRACE_SHADING").is_none() {
            return;
        }
        let mat = self.shading_ui.target.as_ref().and_then(|t| self.shading_read(t));
        let (rim, ramp, width) = mat.as_ref().map_or((0.0, 0.0, 0.0), |m| (m.rim_intensity, m.npr.ramp_blend, m.outline_width));
        let line = format!(
            "shadetrace target={:?} rim={rim:.3} ramp_blend={ramp:.2} outline={width:.4} undo={} redo={} gesture={} script={} {extra}",
            self.shading_ui.target.as_ref().map(|t| format!("{t:?}").replace(' ', "")).unwrap_or_default(),
            if self.shading_ui.source == Source::Modelagem { self.modeling.history_labels().len() } else { self.core.history().undo_depth() },
            self.shading_can_undo_redo().1,
            self.shading_ui.gesture.is_some(),
            match &self.shading_ui.script {
                Some(sc) if sc.is_done() => "done",
                Some(_) => "running",
                None => "none",
            }
        );
        if self.shading_ui.trace.as_deref() != Some(line.as_str()) {
            eprintln!("{line}");
            self.shading_ui.trace = Some(line);
        }
    }

    /// Entrada no workspace: histórico do núcleo mais fundo, Biblioteca aberta, alvo válido.
    pub(crate) fn shading_enter(&mut self) {
        if !self.shading_ui.history_limit_set {
            self.core.set_history_limit(400);
            self.shading_ui.history_limit_set = true;
        }
        if !self.shading_ui.palette_loaded {
            self.shading_load_project_palette();
        }
        let w = self.lib_workspace();
        w.ensure_open();
        self.shading_validate_target();
    }

    /// Vindo de outro workspace, o Shading já abre no material do que o artista estava editando:
    /// seleção do Cenário, objeto ativo da Modelagem ou o material do personagem.
    pub(crate) fn shading_on_enter(&mut self, from: crate::workspace::WorkspaceId) {
        use crate::workspace::WorkspaceId;
        let picked = match from {
            WorkspaceId::Cenario => self.scene_ui.selection.active().cloned().and_then(|id| {
                let slot = self.core.project().scene.node(&id)?;
                if slot.kind == anigo_core::hierarchy::NodeKind::CharacterRoot {
                    self.shading_character_target().map(|t| (Source::Cenario, t))
                } else {
                    Some((Source::Cenario, Target::Node(id)))
                }
            }),
            WorkspaceId::Modelagem => self.modeling.active_object().map(|o| (Source::Modelagem, Target::Object(o.id))),
            WorkspaceId::Personagem => self.shading_character_target().map(|t| (Source::Personagem, t)),
            _ => None,
        };
        if let Some((src, t)) = picked {
            if self.shading_read(&t).is_some() {
                self.shading_ui.source = src;
                self.shading_select(t);
            }
        }
    }

    /// Ferramenta do shelf → grupo do painel (ou modo do preview).
    pub(crate) fn on_shading_tool_selected(&mut self, id: &str) {
        match id {
            "cel_shader" => self.shading_ui.focus_group = Some(Group::Base),
            "rim" => self.shading_ui.focus_group = Some(Group::Rim),
            "outline" => self.shading_ui.focus_group = Some(Group::Outline),
            "palette" => self.shading_ui.palette_open = true,
            "shader_ball" => {
                self.shading_ui.mode = PreviewMode::Ball;
                self.shading_ui.framed = false;
            }
            _ => {}
        }
    }

    /// Ctrl+Z / Ctrl+Shift+Z / Ctrl+Y e Ctrl+C / Ctrl+V (material) no workspace Shading (fora de campos de texto).
    pub(crate) fn shading_shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        if self.hk(ctx, "edit.redo") {
            self.undo_redo(false);
            return;
        }
        if self.hk(ctx, "edit.undo") {
            self.undo_redo(true);
            return;
        }
        if self.hk(ctx, "material.copy") {
            self.shading_copy();
        }
        if self.hk(ctx, "material.paste") {
            self.shading_paste();
        }
        if self.hk(ctx, "material.frame") {
            self.shading_ui.framed = false;
        }
    }

    /// Início do quadro do Shading (antes dos painéis).
    pub(crate) fn shading_frame_begin(&mut self) {
        self.shading_enter();
    }

    /// Fim do quadro do Shading: fecha o gesto de edição e emite o trace (depois dos painéis).
    pub(crate) fn shading_frame_end(&mut self, ctx: &egui::Context) {
        self.shading_finish_gesture(ctx);
        self.shading_trace("");
    }
}
