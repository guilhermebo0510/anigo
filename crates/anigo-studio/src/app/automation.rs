//! Cenários roteirizados (`--scenario <nome>`): põem o app num estado conhecido para
//! screenshots determinísticos (validação visual em CI/nuvem com Xvfb + lavapipe).

use super::StudioApp;
use crate::modeling::{Axis, ModalContext, Mode, PrimitiveKind, TransformKind};
use anigo_modeling::{SelectMode, Selection};
use glam::{Vec2, Vec3};

pub const SCENARIOS: &[&str] = &[
    "edit-faces", "edit-vertices", "tool-move", "tool-rotate", "tool-scale", "grab-modal",
    "sphere-wire", "scene-many", "dense-sphere", "cel-preview", "cel-preview-inactive", "cel-edit", "ops-demo", "ops-bevel", "modifiers", "proportional",
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
            "dense-sphere" => {
                m.add_primitive(PrimitiveKind::UvSphere, Vec3::new(0.0, 1.0, 0.0));
                m.set_mode(Mode::Edit);
            }
            other => {
                eprintln!("cenário desconhecido: {other} (disponíveis: {SCENARIOS:?})");
            }
        }
    }
}
