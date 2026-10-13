//! Cenários roteirizados (`--scenario <nome>`): põem o app num estado conhecido para
//! screenshots determinísticos (validação visual em CI/nuvem com Xvfb + lavapipe).

use super::StudioApp;
use crate::modeling::{Axis, ModalContext, Mode, PrimitiveKind, TransformKind};
use anigo_modeling::{SelectMode, Selection};
use glam::{Vec2, Vec3};

pub const SCENARIOS: &[&str] = &[
    "edit-faces", "edit-vertices", "tool-move", "tool-rotate", "tool-scale", "grab-modal",
    "sphere-wire", "scene-many", "dense-sphere", "cel-preview", "cel-preview-inactive", "cel-edit", "ops-demo", "ops-bevel", "modifiers", "proportional",
    "uv-editor", "uv-sphere", "uv-click",
    "normals-overlay", "normals-transfer", "normals-transfer-clean", "normals-paint",
    "sculpt-ready", "sculpt-draw", "sculpt-mask",
    "hair-ready", "hair-draw", "hair-groups", "hair-cel",
    "character-default", "character-sliders", "character-female", "character-front", "character-face",
];

impl StudioApp {
    pub(crate) fn run_scenario(&mut self) {
        let Some(name) = self.cli.scenario.clone() else {
            return;
        };
        // executa uma única vez, no 3º quadro (UI e GPU já inicializadas)
        if self.frame_count != 3 {
            return;
        }
        if name.starts_with("layout-") {
            self.run_layout_scenario(&name);
            return;
        }
        if crate::library_ui::scenario::is_library_scenario(&name) {
            return; // a máquina de estados do cenário vive em `LibraryWorkspace::scenario_tick`
        }
        if super::scene_ui::is_scene_scenario(&name) {
            self.scene_start_script(&name);
            return; // a máquina de estados vive em `scene_script_tick`
        }
        if super::lighting_ui::is_light_scenario(&name) {
            self.light_start_script(&name);
            return; // a máquina de estados vive em `light_script_tick`
        }
        if super::render_ui::is_render_scenario(&name) {
            self.render_start_script(&name);
            return; // a máquina de estados vive em `render_script_tick`
        }
        if super::shading_ui::is_shading_scenario(&name) {
            self.shading_start_script(&name);
            return; // a máquina de estados vive em `shading_script_tick`
        }
        if super::posing_ui::is_posing_scenario(&name) {
            self.posing_start_script(&name);
            return; // a máquina de estados vive em `posing_script_tick`
        }
        if super::animation_ui::is_animation_scenario(&name) {
            self.anim_start_script(&name);
            return; // a máquina de estados vive em `anim_script_tick`
        }
        if name.starts_with("sculpt-") {
            self.run_sculpt_scenario(&name);
            return;
        }
        if name.starts_with("hair-") {
            self.run_hair_scenario(&name);
            return;
        }
        let m = &mut self.modeling;
        match name.as_str() {
            "edit-faces" => {
                m.set_mode(Mode::Edit);
                m.set_select_mode(SelectMode::Face);
                let mesh = &m.active_object().expect("objeto").mesh;
                let sel = Selection::from_faces(mesh, [0u32, 2]);
                m.set_selection(sel, None);
            }
            "edit-vertices" => {
                m.set_mode(Mode::Edit);
                m.set_select_mode(SelectMode::Vertex);
                m.set_selection(Selection::from_verts([0u32, 1, 2, 5]), Some(2));
            }
            "tool-move" | "tool-rotate" | "tool-scale" => {
                let id = &name["tool-".len()..];
                let idx = crate::app::inspector::MODELAGEM_EDIT.iter().position(|t| t.id == id).unwrap_or(1);
                // ferramentas de transformação são as mesmas nos modos Objeto e Edição, na mesma ordem relativa
                let obj_idx = crate::workspace::MODELAGEM_OBJECT.iter().position(|t| t.id == id).unwrap_or(idx);
                self.tool_idx.insert(self.ws, obj_idx);
            }
            "grab-modal" => {
                m.set_mode(Mode::Edit);
                m.set_select_mode(SelectMode::Face);
                let mesh = &m.active_object().expect("objeto").mesh;
                let sel = Selection::from_faces(mesh, [2u32]);
                m.set_selection(sel, None);
                if m.begin_modal(TransformKind::Grab, Vec2::new(400.0, 300.0)) {
                    if let Some(t) = m.modal_mut() {
                        t.toggle_axis(Axis::Y, false);
                        for ch in "0.8".chars() {
                            t.push_numeric(ch);
                        }
                    }
                    let ctx = ModalContext {
                        camera: self.camera,
                        viewport: Vec2::new(1000.0, 800.0),
                    };
                    m.update_modal(Vec2::ZERO, &ctx);
                    if let Some(s) = m.modal_status(Vec2::ZERO, &ctx) {
                        m.status = s;
                    }
                }
            }
            "sphere-wire" => {
                m.add_primitive(PrimitiveKind::QuadSphere, Vec3::new(0.0, 1.0, 0.0));
                m.set_mode(Mode::Edit);
                m.set_select_mode(SelectMode::Face);
                let mesh = &m.active_object().expect("objeto").mesh;
                let sel = Selection::from_faces(mesh, (0..40u32).collect::<Vec<_>>());
                m.set_selection(sel, None);
                self.camera.distance = 4.5;
            }
            "scene-many" => {
                for (i, k) in PrimitiveKind::ALL.iter().enumerate() {
                    let x = (i as f32 % 5.0 - 2.0) * 3.0;
                    let z = (i as f32 / 5.0).floor() * 3.0 - 1.5;
                    m.add_primitive(*k, Vec3::new(x, 1.0, z));
                }
                self.camera.distance = 16.0;
                self.camera.pitch = 0.6;
            }
            "cel-preview" => {
                m.delete_active_object();
                m.add_primitive(PrimitiveKind::QuadSphere, Vec3::new(0.0, 1.0, 0.0));
                m.add_primitive(PrimitiveKind::Torus, Vec3::new(2.4, 1.0, 0.0));
                m.add_primitive(PrimitiveKind::Cone, Vec3::new(-2.4, 1.0, 0.0));
                self.view_shading = crate::app::ViewShading::Cel;
                self.camera.distance = 9.0;
                if let Some(c) = self.gpu.as_mut().and_then(|g| g.cel.as_mut()) {
                    c.light.direction = glam::Vec3::new(-0.45, 0.6, 0.65).normalize().to_array();
                }
            }
            "cel-preview-inactive" => {
                m.delete_active_object();
                m.add_primitive(PrimitiveKind::QuadSphere, Vec3::new(0.0, 1.0, 0.0));
                m.add_primitive(PrimitiveKind::Torus, Vec3::new(2.4, 1.0, 0.0));
                m.add_primitive(PrimitiveKind::Cone, Vec3::new(-2.4, 1.0, 0.0));
                m.doc.active = None;
                self.view_shading = crate::app::ViewShading::Cel;
                self.camera.distance = 9.0;
                if let Some(c) = self.gpu.as_mut().and_then(|g| g.cel.as_mut()) {
                    c.light.direction = glam::Vec3::new(-0.45, 0.6, 0.65).normalize().to_array();
                }
            }
            "ops-demo" => {
                use crate::app::operators::{apply_kind, OpKind, OpParams};
                m.set_mode(Mode::Edit);
                m.set_select_mode(SelectMode::Face);
                let top = Selection::from_faces(&m.active_object().expect("objeto").mesh, [2u32]);
                m.set_selection(top, None);
                let p = |f: &dyn Fn(&mut OpParams)| {
                    let mut p = OpParams::default();
                    f(&mut p);
                    p
                };
                let steps: [(OpKind, OpParams); 4] = [
                    (OpKind::Extrude, p(&|q| q.distance = 0.9)),
                    (OpKind::Inset, p(&|q| q.thickness = 0.28)),
                    (OpKind::Extrude, p(&|q| q.distance = -0.35)),
                    (OpKind::Inset, p(&|q| q.thickness = 0.1)),
                ];
                for (k, params) in steps {
                    if let Err(e) = m.edit_op(k.label(), |mesh, sel| apply_kind(k, &params, mesh, sel)) {
                        eprintln!("ops-demo: {e}");
                    }
                }
                self.camera.distance = 8.5;
            }
            "proportional" => {
                m.delete_active_object();
                m.add_primitive(PrimitiveKind::Grid, Vec3::new(0.0, 0.0, 0.0));
                m.set_mode(Mode::Edit);
                m.set_select_mode(SelectMode::Vertex);
                let center = {
                    let mesh = &m.active_object().expect("objeto").mesh;
                    (0..mesh.vert_count() as u32).find(|&v| mesh.co(v).length() < 1e-5)
                };
                if let Some(c) = center {
                    m.set_selection(Selection::from_verts([c]), Some(c));
                    m.proportional.enabled = true;
                    m.proportional.radius = 1.6;
                    if m.begin_modal(TransformKind::Grab, Vec2::ZERO) {
                        if let Some(t) = m.modal_mut() {
                            t.toggle_axis(Axis::Y, false);
                            t.push_numeric('1');
                            t.push_numeric('.');
                            t.push_numeric('2');
                        }
                        let ctx = ModalContext { camera: self.camera, viewport: Vec2::new(1000.0, 800.0) };
                        m.update_modal(Vec2::ZERO, &ctx);
                        m.confirm_modal();
                    }
                }
                self.camera.pitch = 0.45;
                self.camera.target = Vec3::new(0.0, 0.4, 0.0);
            }
            "modifiers" => {
                use anigo_modeling::modifiers::{mirror::MirrorParams, stack::{Modifier, ModifierEntry, SubsurfSettings}};
                use crate::app::operators::{apply_kind, OpKind, OpParams};
                // meia caixa com chanfro → Mirror X → Subsurf: cabeça de "capsule" low poly
                m.set_mode(Mode::Edit);
                m.set_select_mode(SelectMode::Edge);
                m.select_all();
                let params = OpParams { width: 0.25, segments: 2, ..OpParams::default() };
                let _ = m.edit_op("Chanfro", |mesh, sel| apply_kind(OpKind::Bevel, &params, mesh, sel));
                m.select_none();
                m.set_mode(Mode::Object);
                if let Some(o) = m.active_object_mut() {
                    o.mesh.transform_positions(|p| p * glam::Vec3::new(1.0, 1.0, 1.0));
                    o.modifiers.push(ModifierEntry::new(Modifier::Subsurf(SubsurfSettings { levels_viewport: 2, ..Default::default() })));
                    o.modifiers.push(ModifierEntry::new(Modifier::Mirror(MirrorParams { axes: [false, false, true], ..Default::default() })));
                }
                m.revision += 1;
                self.view_shading = crate::app::ViewShading::Cel;
            }
            "ops-bevel" => {
                use crate::app::operators::{apply_kind, OpKind, OpParams};
                m.set_mode(Mode::Edit);
                m.set_select_mode(SelectMode::Edge);
                m.select_all();
                let params = OpParams { width: 0.22, segments: 4, ..OpParams::default() };
                if let Err(e) = m.edit_op("Chanfro", |mesh, sel| apply_kind(OpKind::Bevel, &params, mesh, sel)) {
                    eprintln!("ops-bevel: {e}");
                }
                m.select_none();
                self.view_shading = crate::app::ViewShading::Cel;
            }
            "cel-edit" => {
                m.delete_active_object();
                m.add_primitive(PrimitiveKind::Capsule, Vec3::new(0.0, 1.0, 0.0));
                m.set_mode(Mode::Edit);
                m.set_select_mode(SelectMode::Face);
                let mesh = &m.active_object().expect("objeto").mesh;
                let sel = Selection::from_faces(mesh, (0..30u32).collect::<Vec<_>>());
                m.set_selection(sel, None);
                self.view_shading = crate::app::ViewShading::Cel;
            }
            "uv-editor" | "uv-click" => {
                // cubo com costuras de cruz -> unwrap -> editor de UV aberto com xadrez
                m.set_mode(Mode::Edit);
                m.set_select_mode(SelectMode::Face);
                self.uv_seed_tree_seams();
                self.modeling.set_select_mode(SelectMode::Face);
                self.modeling.select_none();
                self.run_uv_action(crate::app::uv_ui::UvAction::Unwrap);
                self.show_uv_editor = true;
                self.uv.checker = crate::app::uv_ui::CheckerMode::C8;
                if name == "uv-editor" {
                    let mesh = &self.modeling.active_object().expect("objeto").mesh;
                    let sel = Selection::from_faces(mesh, [2u32]);
                    self.modeling.set_selection(sel, None);
                }
            }
            "uv-sphere" => {
                self.modeling.delete_active_object();
                self.modeling.add_primitive(PrimitiveKind::QuadSphere, Vec3::new(0.0, 1.0, 0.0));
                self.modeling.set_mode(Mode::Edit);
                self.modeling.set_select_mode(SelectMode::Face);
                self.uv.params.method = anigo_modeling::uv::unwrap::UnwrapMethod::LscmArap;
                self.run_uv_action(crate::app::uv_ui::UvAction::AutoSeamsCharacter);
                self.run_uv_action(crate::app::uv_ui::UvAction::Unwrap);
                self.show_uv_editor = true;
                self.uv.color_mode = crate::app::uv_ui::ColorMode::Distortion;
                let mesh = &self.modeling.active_object().expect("objeto").mesh;
                let sel = Selection::from_faces(mesh, (0..12u32).collect::<Vec<_>>());
                self.modeling.set_selection(sel, None);
                self.camera.distance = 4.5;
            }
            "dense-sphere" => {
                m.add_primitive(PrimitiveKind::UvSphere, Vec3::new(0.0, 1.0, 0.0));
                m.set_mode(Mode::Edit);
            }
            other if other.starts_with("normals-") && self.scenario_normals(other) => {}
            "character-default" => {
                self.ws = crate::workspace::WorkspaceId::Personagem;
                self.tool_idx.insert(self.ws, 0);
            }
            "character-sliders" => {
                use crate::character::{macros::{self, Proportion}, sliders};
                use anigo_core::somatotype::SomatotypeCoords;
                self.ws = crate::workspace::WorkspaceId::Personagem;
                self.tool_idx.insert(self.ws, 0);
                let eyes = anigo_core::morph_catalog::find_slider_def("eye_scale_uniform");
                let mut cmds = vec![
                    Proportion::Height.command(1.2),
                    Proportion::Legs.command(1.15),
                    macros::somatotype_command(SomatotypeCoords::new(0.1, 0.75, 0.15)),
                ];
                cmds.extend(eyes.map(|d| sliders::slider_command(d, 1.4)));
                for c in cmds {
                    if let Err(e) = self.core.apply(c) {
                        eprintln!("character-sliders: {e}");
                    }
                }
                self.char_ui.framed = false; // reenquadra a nova altura
            }
            "character-front" => {
                // vista frontal sem perspectiva inclinada: base da comparação com o frame canônico headless
                self.ws = crate::workspace::WorkspaceId::Personagem;
                self.tool_idx.insert(self.ws, 0);
                self.char_ui.camera.yaw = 0.0;
                self.char_ui.camera.pitch = 0.0;
            }
            "character-face" => {
                use crate::character::sliders;
                self.ws = crate::workspace::WorkspaceId::Personagem;
                self.tool_idx.insert(self.ws, 1);
                for (id, v) in [("eye_scale_uniform", 1.35), ("jaw_v_line_taper", 0.7)] {
                    if let Some(d) = anigo_core::morph_catalog::find_slider_def(id) {
                        let _ = self.core.apply(sliders::slider_command(d, v));
                    }
                }
                self.char_ui.camera.yaw = 0.25;
                self.char_ui.camera.pitch = 0.0;
                let b = self.gpu.as_ref().and_then(|g| g.cel.as_ref()).and_then(|c| c.character_bounds);
                if let Some((lo, hi)) = b {
                    super::character_ui::frame_face(&mut self.char_ui.camera, lo, hi);
                }
            }
            "character-female" => {
                self.ws = crate::workspace::WorkspaceId::Personagem;
                self.tool_idx.insert(self.ws, 0);
                match crate::character::macros::set_gender(self.core.project(), anigo_core::mesh::BaseGender::Female) {
                    Ok(Some(c)) => {
                        if let Err(e) = self.core.apply(c) {
                            eprintln!("character-female: {e}");
                        }
                    }
                    other => eprintln!("character-female: {other:?}"),
                }
            }
            other => {
                eprintln!("cenário desconhecido: {other} (disponíveis: {SCENARIOS:?})");
            }
        }
    }

    /// Cenários do Modo Cabelo: cabeça de teste (esfera) + traços roteirizados que rodam no primeiro
    /// quadro do viewport pelo mesmo caminho do mouse (`HairState::run_script`).
    fn run_hair_scenario(&mut self, name: &str) {
        use crate::modeling::hair::{HairTool, ScriptStep};
        let m = &mut self.modeling;
        m.delete_active_object();
        m.add_primitive(PrimitiveKind::UvSphere, Vec3::new(0.0, 1.2, 0.0));
        if let Some(o) = m.active_object_mut() {
            o.name = "Cabeça de teste".into();
            o.mesh = anigo_modeling::primitives::uv_sphere(1.0, 32, 64);
            o.mesh.name = o.name.clone();
            // pele (preview Solid/Cel) para o cabelo contrastar
            o.material = Some(crate::modeling::ObjectMaterial {
                name: "Pele".into(),
                base_color: [0.80, 0.56, 0.46],
                shade_color: [0.52, 0.32, 0.32],
                highlight_color: [1.0, 0.95, 0.9],
            });
        }
        m.clear_history();
        m.revision += 1;
        m.set_mode(Mode::Hair);
        self.camera.target = Vec3::new(0.0, 1.2, 0.0);
        self.camera.distance = 5.2;
        self.camera.yaw = 0.25;
        self.camera.pitch = 0.18;
        let tool = if name == "hair-groups" { HairTool::Edit } else { HairTool::Draw };
        self.tool_idx.insert(self.ws, tool.index());
        self.hair.tool = tool;
        // ponto na esfera de raio 1 (espaço local) em (longitude, latitude); `out`/`drop` levam o ponto
        // para fora da silhueta (o traço continua num plano paralelo à tela)
        let sph = |lon: f32, lat: f32| Vec3::new(lat.cos() * lon.sin(), lat.sin(), lat.cos() * lon.cos());
        let lock = |lon: f32, lat0: f32, lat1: f32, out: f32, drop: f32| -> Vec<Vec3> {
            let mut pts: Vec<Vec3> = (0..=10).map(|k| sph(lon, lat0 + (lat1 - lat0) * k as f32 / 10.0)).collect();
            let end = sph(lon, lat1);
            let dir = Vec3::new(end.x, 0.0, end.z).normalize_or_zero();
            for k in 1..=5 {
                let t = k as f32 / 5.0;
                pts.push(end + dir * (out * t) - Vec3::Y * (drop * t));
            }
            pts
        };
        let mut steps: Vec<ScriptStep> = Vec::new();
        match name {
            // sem traços: o smoke de entrada real (xdotool) desenha a mecha
            "hair-ready" => {}
            "hair-draw" => {
                steps.push(ScriptStep::NewGroup("Franja"));
                steps.push(ScriptStep::Symmetry(true));
                for (lon, lat1, drop) in [(0.18, 0.55, 0.3), (0.40, 0.45, 0.35), (0.62, 0.3, 0.5), (0.85, 0.1, 0.7)] {
                    steps.push(ScriptStep::Draw(lock(lon, 1.35, lat1, 0.10, drop)));
                }
                steps.push(ScriptStep::SelectLast(Some(2)));
            }
            _ => {
                steps.push(ScriptStep::NewGroup("Franja"));
                steps.push(ScriptStep::Symmetry(true));
                for (lon, lat1, drop) in [(0.15, 0.6, 0.25), (0.38, 0.5, 0.3), (0.6, 0.4, 0.4)] {
                    steps.push(ScriptStep::Draw(lock(lon, 1.35, lat1, 0.06, drop)));
                }
                steps.push(ScriptStep::NewGroup("Lateral"));
                steps.push(ScriptStep::Symmetry(true));
                for (lon, lat0, drop) in [(1.15, 1.0, 1.0), (1.4, 0.8, 1.3), (1.65, 0.7, 1.2)] {
                    steps.push(ScriptStep::Draw(lock(lon, lat0, -0.1, 0.22, drop)));
                }
                steps.push(ScriptStep::NewGroup("Topete"));
                for (lon, drop) in [(-0.06, 0.5), (0.1, 0.55)] {
                    steps.push(ScriptStep::Draw(lock(lon, 1.5, 0.95, 0.12, drop)));
                }
                steps.push(ScriptStep::SelectLast(Some(1)));
            }
        }
        if name == "hair-cel" {
            steps.push(ScriptStep::LeaveHairMode);
        }
        self.hair.script = Some(steps);
        if name == "hair-cel" {
            self.view_shading = crate::app::ViewShading::Cel;
            self.camera.yaw = 0.35;
            if let Some(c) = self.gpu.as_mut().and_then(|g| g.cel.as_mut()) {
                c.light.direction = glam::Vec3::new(-0.45, 0.6, 0.65).normalize().to_array();
            }
        }
    }

    /// Cenários do Modo Escultura: esfera densa, modo Escultura e (conforme o cenário) traços roteirizados
    /// que rodam no próximo `sculpt_input` pelo mesmo caminho do mouse.
    fn run_sculpt_scenario(&mut self, name: &str) {
        use crate::app::sculpt_ui::scripted;
        use crate::modeling::sculpt::tool_index_for_brush;
        use anigo_modeling::sculpt::brush::BrushKind;
        let m = &mut self.modeling;
        m.delete_active_object();
        m.add_primitive(PrimitiveKind::QuadSphere, Vec3::new(0.0, 1.0, 0.0));
        if let Some(o) = m.active_object_mut() {
            o.mesh = anigo_modeling::primitives::quad_sphere(1.0, 40);
        }
        m.clear_history();
        m.revision += 1;
        m.set_mode(Mode::Sculpt);
        self.camera.distance = 4.2;
        let brush = match name {
            "sculpt-mask" => BrushKind::MaskPaint,
            _ => BrushKind::Draw,
        };
        if let Some(i) = tool_index_for_brush(brush) {
            self.tool_idx.insert(self.ws, i);
        }
        self.sculpt.active = brush;
        match name {
            "sculpt-draw" => {
                self.sculpt.slot_mut(BrushKind::Draw).brush.strength = 0.06;
                self.sculpt.slot_mut(BrushKind::Draw).radius_px = 70.0;
                self.sculpt.slot_mut(BrushKind::Inflate).brush.strength = 0.1;
                self.sculpt.symmetry = anigo_modeling::sculpt::symmetry::Symmetry::mirror_x();
                // zigue-zague de Draw + um toque de Inflate; o círculo fica sobre a esfera
                self.sculpt.script = Some(scripted(
                    BrushKind::Draw,
                    false,
                    vec![
                        vec![(0.40, 0.40), (0.47, 0.50), (0.40, 0.60)],
                        vec![(0.52, 0.34), (0.58, 0.46), (0.52, 0.58)],
                    ],
                    Some((0.56, 0.68)),
                ));
                self.sculpt.followup = Some(scripted(BrushKind::Inflate, false, vec![vec![(0.47, 0.30), (0.50, 0.31)]], Some((0.56, 0.68))));
            }
            "sculpt-mask" => {
                self.sculpt.slot_mut(BrushKind::MaskPaint).brush.strength = 1.0;
                self.sculpt.slot_mut(BrushKind::MaskPaint).radius_px = 60.0;
                self.sculpt.script = Some(scripted(
                    BrushKind::MaskPaint,
                    false,
                    vec![vec![(0.38, 0.42), (0.46, 0.50), (0.38, 0.58)], vec![(0.46, 0.36), (0.50, 0.36)]],
                    Some((0.58, 0.55)),
                ));
                self.sculpt.followup = Some(scripted(BrushKind::Draw, false, vec![vec![(0.28, 0.50), (0.52, 0.50)]], Some((0.58, 0.55))));
            }
            _ => {}
        }
    }
}
