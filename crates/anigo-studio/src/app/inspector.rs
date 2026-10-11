//! Painel direito (Inspector) do workspace Modelagem + tabelas de ferramentas por modo.

use super::StudioApp;
use crate::i18n::tr;
use crate::icons;
use crate::modeling::{Mode, PrimitiveKind};
use crate::theme;
use crate::workspace::{ToolDef, MODELAGEM_OBJECT};
use anigo_modeling::SelectMode;
use egui::RichText;
use glam::{EulerRot, Quat};

const fn tool(id: &'static str, icon: &'static str, label_key: &'static str) -> ToolDef {
    ToolDef { id, icon, label_key }
}

pub const MODELAGEM_EDIT: &[ToolDef] = &[
    tool("select_box", icons::SELECTION, "tool.select_box"),
    tool("move", icons::ARROWS_OUT_CARDINAL, "tool.move"),
    tool("rotate", icons::ARROWS_CLOCKWISE, "tool.rotate"),
    tool("scale", icons::ARROWS_OUT, "tool.scale"),
    tool("extrude", icons::ARROW_CLOCKWISE, "tool.extrude"),
    tool("inset", icons::SQUARES_FOUR, "tool.inset"),
    tool("bevel", icons::POLYGON, "tool.bevel"),
    tool("loop_cut", icons::LINE_SEGMENTS, "tool.loop_cut"),
    tool("knife", icons::KNIFE, "tool.knife"),
    tool("bridge", icons::PIPE, "tool.bridge"),
    tool("merge", icons::UNITE, "tool.merge"),
    tool("fill", icons::PAINT_BUCKET, "tool.fill"),
    tool("dissolve", icons::EXCLUDE, "tool.dissolve"),
    tool("smooth_verts", icons::SPIRAL, "tool.smooth_verts"),
];

pub const MODELAGEM_SCULPT: &[ToolDef] = &[
    tool("sculpt.draw", icons::PENCIL_SIMPLE, "sculpt.draw"),
    tool("sculpt.clay", icons::HAND_GRABBING, "sculpt.clay"),
    tool("sculpt.clay_strips", icons::LINE_SEGMENT, "sculpt.clay_strips"),
    tool("sculpt.smooth", icons::DROP, "sculpt.smooth"),
    tool("sculpt.flatten", icons::RULER, "sculpt.flatten"),
    tool("sculpt.inflate", icons::SPHERE, "sculpt.inflate"),
    tool("sculpt.pinch", icons::ARROWS_IN, "sculpt.pinch"),
    tool("sculpt.grab", icons::HAND_POINTING, "sculpt.grab"),
    tool("sculpt.snake_hook", icons::BEZIER_CURVE, "sculpt.snake_hook"),
    tool("sculpt.crease", icons::SNOWFLAKE, "sculpt.crease"),
    tool("sculpt.mask", icons::CIRCLE, "sculpt.mask"),
];

pub fn modeling_tools(mode: Mode) -> &'static [ToolDef] {
    match mode {
        Mode::Object => MODELAGEM_OBJECT,
        Mode::Edit => MODELAGEM_EDIT,
        Mode::Sculpt => MODELAGEM_SCULPT,
    }
}

fn section(ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut egui::Ui)) {
    egui::CollapsingHeader::new(RichText::new(title).strong())
        .default_open(true)
        .show(ui, add);
}

impl StudioApp {
    pub(crate) fn inspector_modeling(&mut self, ui: &mut egui::Ui) {
        egui::Panel::right("inspector")
            .default_size(300.0)
            .size_range(240.0..=460.0)
            .show(ui, |ui| {
                ui.add_space(6.0);
                ui.label(RichText::new(tr("panel.inspector")).strong().size(14.0));
                ui.separator();
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    match self.modeling.mode {
                        Mode::Object => self.inspector_object(ui),
                        Mode::Edit => self.inspector_edit(ui),
                        Mode::Sculpt => self.inspector_sculpt(ui),
                    }
                    section(ui, tr("panel.history"), |ui| {
                        let labels = self.modeling.history_labels();
                        if labels.is_empty() {
                            ui.label(RichText::new("—").color(theme::TEXT_DIM));
                        }
                        for l in labels.iter().rev().take(8) {
                            ui.label(RichText::new(*l).size(12.0).color(theme::TEXT_DIM));
                        }
                    });
                });
            });
    }

    fn inspector_object(&mut self, ui: &mut egui::Ui) {
        section(ui, "Cena", |ui| {
            let mut pick = None;
            for (i, o) in self.modeling.doc.objects.iter().enumerate() {
                let active = self.modeling.doc.active == Some(i);
                if ui
                    .selectable_label(active, format!("{}  {}", icons::CUBE, o.name))
                    .clicked()
                {
                    pick = Some(i);
                }
            }
            if let Some(i) = pick {
                self.modeling.set_active(i);
            }
            if self.modeling.doc.objects.is_empty() {
                ui.label(RichText::new("Cena vazia — adicione uma primitiva").color(theme::TEXT_DIM));
            }
        });
        section(ui, tr("action.add"), |ui| {
            ui.horizontal_wrapped(|ui| {
                for kind in PrimitiveKind::ALL {
                    if ui.button(kind.label()).clicked() {
                        let at = glam::Vec3::new(self.camera.target.x, 0.0, self.camera.target.z);
                        let at = glam::Vec3::new(at.x, 1.0, at.z);
                        self.modeling.add_primitive(kind, at);
                    }
                }
            });
        });
        let mut want_delete = false;
        let mut want_duplicate = false;
        if self.modeling.active_object().is_some() {
            section(ui, "Objeto ativo", |ui| {
                // Nome
                let mut name = self.modeling.active_object().map(|o| o.name.clone()).unwrap_or_default();
                if ui.text_edit_singleline(&mut name).changed() {
                    if let Some(o) = self.modeling.active_object_mut() {
                        o.name = name;
                    }
                }
                let mut checkpointed = false;
                let mut start = |s: &mut crate::modeling::ModelingSession, resp: &egui::Response, label: &str| {
                    if (resp.drag_started() || resp.gained_focus()) && !checkpointed {
                        s.checkpoint(label);
                        checkpointed = true;
                    }
                };
                if let Some(obj) = self.modeling.active_object() {
                    let mut t = obj.transform.translation;
                    let (ex, ey, ez) = obj.transform.rotation.to_euler(EulerRot::XYZ);
                    let clean = |v: f32| if v.abs() < 1e-4 { 0.0 } else { v };
                    let mut euler = glam::Vec3::new(clean(ex.to_degrees()), clean(ey.to_degrees()), clean(ez.to_degrees()));
                    let mut s = obj.transform.scale;
                    let (mut ct, mut cr, mut cs) = (false, false, false);
                    let mut responses: Vec<(&str, egui::Response)> = Vec::new();
                    egui::Grid::new("xform").num_columns(4).spacing([4.0, 4.0]).show(ui, |ui| {
                        ui.label("Posição");
                        for v in [&mut t.x, &mut t.y, &mut t.z] {
                            let r = ui.add(egui::DragValue::new(v).speed(0.02).fixed_decimals(3));
                            ct |= r.changed();
                            responses.push(("Mover", r));
                        }
                        ui.end_row();
                        ui.label("Rotação");
                        for v in [&mut euler.x, &mut euler.y, &mut euler.z] {
                            let r = ui.add(egui::DragValue::new(v).speed(0.5).suffix("°").fixed_decimals(1));
                            cr |= r.changed();
                            responses.push(("Rotacionar", r));
                        }
                        ui.end_row();
                        ui.label("Escala");
                        for v in [&mut s.x, &mut s.y, &mut s.z] {
                            let r = ui.add(egui::DragValue::new(v).speed(0.01).fixed_decimals(3));
                            cs |= r.changed();
                            responses.push(("Escalar", r));
                        }
                        ui.end_row();
                    });
                    for (label, r) in &responses {
                        start(&mut self.modeling, r, label);
                    }
                    if ct || cr || cs {
                        if let Some(o) = self.modeling.active_object_mut() {
                            if ct {
                                o.transform.translation = t;
                            }
                            if cr {
                                o.transform.rotation = Quat::from_euler(
                                    EulerRot::XYZ,
                                    euler.x.to_radians(),
                                    euler.y.to_radians(),
                                    euler.z.to_radians(),
                                );
                            }
                            if cs {
                                o.transform.scale = s.max(glam::Vec3::splat(1e-3));
                            }
                        }
                        self.modeling.revision += 1;
                    }
                }
                ui.horizontal(|ui| {
                    if ui.button(format!("{}  Duplicar", icons::SQUARES_FOUR)).clicked() {
                        want_duplicate = true;
                    }
                    if ui.button(format!("{}  {}", icons::TRASH, tr("action.delete"))).clicked() {
                        want_delete = true;
                    }
                });
            });
        }
        if want_duplicate {
            self.modeling.duplicate_active_object();
        }
        if want_delete {
            self.modeling.delete_active_object();
        }
        self.inspector_snap(ui);
    }

    fn inspector_snap(&mut self, ui: &mut egui::Ui) {
        section(ui, "Snap", |ui| {
            ui.checkbox(&mut self.modeling.snap_enabled, "Snap por incremento (Ctrl)");
        });
    }

    fn inspector_edit(&mut self, ui: &mut egui::Ui) {
        section(ui, "Seleção", |ui| {
            ui.horizontal(|ui| {
                for (m, key, icon) in [
                    (SelectMode::Vertex, "select.vertex", icons::DOT_OUTLINE),
                    (SelectMode::Edge, "select.edge", icons::LINE_SEGMENT),
                    (SelectMode::Face, "select.face", icons::POLYGON),
                ] {
                    if ui
                        .selectable_label(self.modeling.select_mode == m, format!("{icon} {}", tr(key)))
                        .clicked()
                    {
                        self.modeling.set_select_mode(m);
                    }
                }
            });
            ui.horizontal_wrapped(|ui| {
                if ui.button("Tudo (A)").clicked() {
                    self.modeling.select_all();
                }
                if ui.button("Nada").clicked() {
                    self.modeling.select_none();
                }
                if ui.button("Inverter").clicked() {
                    self.modeling.select_invert();
                }
                if ui.button("Mais").clicked() {
                    self.modeling.select_more();
                }
                if ui.button("Menos").clicked() {
                    self.modeling.select_less();
                }
                if ui.button("Ligados").clicked() {
                    self.modeling.select_linked();
                }
            });
        });
        section(ui, "Edição proporcional (O)", |ui| {
            ui.checkbox(&mut self.modeling.proportional.enabled, "Ativar");
            ui.add_enabled(
                self.modeling.proportional.enabled,
                egui::Slider::new(&mut self.modeling.proportional.radius, 0.05..=10.0)
                    .logarithmic(true)
                    .text("Raio"),
            );
        });
        self.inspector_edit_operators(ui);
        self.inspector_normals(ui);
        self.inspector_uv(ui);
        self.inspector_snap(ui);
        section(ui, "Overlays", |ui| {
            ui.checkbox(&mut self.settings.show_grid, "Grid");
            ui.checkbox(&mut self.modeling.xray, "Raio-X (Alt+Z)");
        });
    }

    fn inspector_sculpt(&mut self, ui: &mut egui::Ui) {
        self.inspector_sculpt_panel(ui);
    }
}
