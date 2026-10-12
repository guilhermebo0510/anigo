//! Cenários roteirizados (`--scenario <nome>`): põem o app num estado conhecido para
//! screenshots determinísticos (validação visual em CI/nuvem com Xvfb + lavapipe).

use super::StudioApp;
use crate::modeling::{Axis, ModalContext, Mode, PrimitiveKind, TransformKind};
use anigo_modeling::{SelectMode, Selection};
use glam::{Vec2, Vec3};

pub const SCENARIOS: &[&str] = &[
    "edit-faces", "edit-vertices", "tool-move", "tool-rotate", "tool-scale", "grab-modal",
    "sphere-wire", "scene-many", "dense-sphere", "cel-preview", "cel-preview-inactive", "cel-edit", "ops-demo", "ops-bevel",
    "sculpt-ready", "sculpt-draw", "sculpt-mask",
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
        if name.starts_with("sculpt-") {
            self.run_sculpt_scenario(&name);
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
            "dense-sphere" => {
                m.add_primitive(PrimitiveKind::UvSphere, Vec3::new(0.0, 1.0, 0.0));
                m.set_mode(Mode::Edit);
            }
            other => {
                eprintln!("cenário desconhecido: {other} (disponíveis: {SCENARIOS:?})");
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
