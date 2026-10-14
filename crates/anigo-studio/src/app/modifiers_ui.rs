//! Seção "Modificadores" do Inspector (pilha não destrutiva do motor).

use super::StudioApp;
use crate::icons;
use crate::theme;
use anigo_modeling::modifiers::{
    array::ArrayParams,
    decimate::DecimateSettings,
    displace::{DisplaceParams, DisplaceSource},
    edge_split::EdgeSplitParams,
    mirror::MirrorParams,
    solidify::SolidifyParams,
    stack::{Modifier, ModifierEntry, SubsurfSettings, TriangulateParams},
    weld::WeldParams,
};
use egui::RichText;

/// Catálogo do menu "Adicionar modificador".
pub fn catalog() -> Vec<(&'static str, Modifier)> {
    vec![
        ("Subdivision Surface", Modifier::Subsurf(SubsurfSettings::default())),
        ("Mirror", Modifier::Mirror(MirrorParams::default())),
        ("Solidify", Modifier::Solidify(SolidifyParams::default())),
        ("Array", Modifier::Array(ArrayParams::default())),
        ("Weld", Modifier::Weld(WeldParams::default())),
        ("Edge Split", Modifier::EdgeSplit(EdgeSplitParams::default())),
        ("Displace", Modifier::Displace(DisplaceParams::default())),
        ("Decimate", Modifier::Decimate(DecimateSettings::default())),
        ("Triangulate", Modifier::Triangulate(TriangulateParams::default())),
    ]
}

enum Action {
    Remove(usize),
    Move(usize, usize),
}

impl StudioApp {
    pub(crate) fn inspector_modifiers(&mut self, ui: &mut egui::Ui) {
        if self.modeling.active_object().is_none() {
            return;
        }
        crate::app::props_tabs::header(ui, RichText::new(crate::i18n::tr("panel.modifiers")).strong())
            .default_open(true)
            .show(ui, |ui| {
                let mut add: Option<Modifier> = None;
                ui.menu_button(format!("{}  Adicionar modificador", icons::PLUS), |ui| {
                    for (name, m) in catalog() {
                        if ui.button(name).clicked() {
                            add = Some(m);
                            ui.close();
                        }
                    }
                });
                if let Some(m) = add {
                    self.modeling.checkpoint("Adicionar modificador");
                    if let Some(o) = self.modeling.active_object_mut() {
                        o.modifiers.push(ModifierEntry::new(m));
                    }
                    self.modeling.revision += 1;
                }

                let count = self.modeling.active_object().map(|o| o.modifiers.modifiers.len()).unwrap_or(0);
                let mut action: Option<Action> = None;
                let mut changed_any = false;
                let mut checkpoint_label: Option<String> = None;
                let mut entries = self.modeling.active_object().map(|o| o.modifiers.modifiers.clone()).unwrap_or_default();
                for (i, entry) in entries.iter_mut().enumerate() {
                    egui::Frame::group(ui.style()).show(ui, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(RichText::new(&entry.name).strong()).on_hover_text(&entry.name);
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.small_button(icons::TRASH).on_hover_text("Remover").clicked() {
                                    action = Some(Action::Remove(i));
                                }
                                if ui.add_enabled(i + 1 < count, egui::Button::new(icons::ARROW_CLOCKWISE).small()).on_hover_text("Descer").clicked() {
                                    action = Some(Action::Move(i, i + 1));
                                }
                                if ui.add_enabled(i > 0, egui::Button::new(icons::ARROW_COUNTER_CLOCKWISE).small()).on_hover_text("Subir").clicked() {
                                    action = Some(Action::Move(i, i - 1));
                                }
                                changed_any |= ui.toggle_value(&mut entry.enabled_render, icons::CAMERA).on_hover_text("Render").changed();
                                changed_any |= ui.toggle_value(&mut entry.enabled_viewport, icons::EYE).on_hover_text("Viewport").changed();
                            });
                        });
                        let (changed, started) = modifier_params(ui, &mut entry.modifier);
                        changed_any |= changed;
                        if started && checkpoint_label.is_none() {
                            checkpoint_label = Some(format!("Editar {}", entry.name));
                        }
                    });
                }
                if let Some(l) = checkpoint_label {
                    self.modeling.checkpoint(&l);
                }
                if changed_any {
                    if let Some(o) = self.modeling.active_object_mut() {
                        o.modifiers.modifiers = entries;
                    }
                    self.modeling.revision += 1;
                }
                match action {
                    Some(Action::Remove(i)) => {
                        self.modeling.checkpoint("Remover modificador");
                        if let Some(o) = self.modeling.active_object_mut() {
                            let _ = o.modifiers.remove(i);
                        }
                        self.modeling.revision += 1;
                    }
                    Some(Action::Move(a, b)) => {
                        self.modeling.checkpoint("Reordenar modificadores");
                        if let Some(o) = self.modeling.active_object_mut() {
                            let _ = o.modifiers.move_entry(a, b);
                        }
                        self.modeling.revision += 1;
                    }
                    None => {}
                }
                if count > 0 {
                    ui.horizontal_wrapped(|ui| {
                        if ui.button("Aplicar tudo").on_hover_text("Funde o resultado na malha base (com undo)").clicked() {
                            match self.modeling.apply_modifier_stack() {
                                Ok(()) => self.modeling.status = "Modificadores aplicados".into(),
                                Err(e) => self.modeling.status = format!("Aplicar: {e}"),
                            }
                        }
                    });
                }
                if count == 0 {
                    ui.label(RichText::new("Nenhum modificador").color(theme::TEXT_DIM).size(12.0));
                }
            });
    }
}

fn dv(ui: &mut egui::Ui, label: &str, v: &mut f32, speed: f64) -> (bool, bool) {
    ui.horizontal_wrapped(|ui| {
        ui.label(label);
        let r = ui.add(egui::DragValue::new(v).speed(speed).fixed_decimals(3));
        (r.changed(), r.drag_started() || r.gained_focus())
    })
    .inner
}

fn du(ui: &mut egui::Ui, label: &str, v: &mut u32, range: std::ops::RangeInclusive<u32>) -> (bool, bool) {
    ui.horizontal_wrapped(|ui| {
        ui.label(label);
        let r = ui.add(egui::DragValue::new(v).range(range));
        (r.changed(), r.drag_started() || r.gained_focus())
    })
    .inner
}

fn cb(ui: &mut egui::Ui, v: &mut bool, label: &str) -> (bool, bool) {
    let r = ui.checkbox(v, label);
    (r.changed(), r.changed())
}

/// Widgets de parâmetros. Devolve `(mudou, começou_edição)`.
fn modifier_params(ui: &mut egui::Ui, m: &mut Modifier) -> (bool, bool) {
    let mut changed = false;
    let mut started = false;
    let mut acc = |r: (bool, bool)| {
        changed |= r.0;
        started |= r.1;
    };
    match m {
        Modifier::Subsurf(s) => {
            acc(du(ui, "Níveis (viewport)", &mut s.levels_viewport, 0..=6));
            acc(du(ui, "Níveis (render)", &mut s.levels_render, 0..=6));
            acc(cb(ui, &mut s.params.use_creases, "Vincos"));
        }
        Modifier::Mirror(p) => {
            ui.horizontal_wrapped(|ui| {
                for (i, n) in ["X", "Y", "Z"].iter().enumerate() {
                    acc(cb(ui, &mut p.axes[i], n));
                }
            });
            acc(cb(ui, &mut p.merge, "Soldar no plano"));
            acc(dv(ui, "Distância de solda", &mut p.merge_distance, 0.001));
            acc(cb(ui, &mut p.clip, "Clipping"));
            ui.horizontal_wrapped(|ui| {
                ui.label("Bisect");
                for (i, n) in ["X", "Y", "Z"].iter().enumerate() {
                    acc(cb(ui, &mut p.bisect[i], n));
                }
            });
        }
        Modifier::Solidify(p) => {
            acc(dv(ui, "Espessura", &mut p.thickness, 0.005));
            if p.thickness == 0.0 {
                p.thickness = 0.001;
            }
            acc(dv(ui, "Offset", &mut p.offset, 0.01));
            p.offset = p.offset.clamp(-1.0, 1.0);
            acc(cb(ui, &mut p.even_thickness, "Espessura uniforme"));
            acc(cb(ui, &mut p.rim_fill, "Preencher borda"));
        }
        Modifier::Array(p) => {
            acc(du(ui, "Quantidade", &mut p.count, 1..=256));
            ui.horizontal_wrapped(|ui| {
                ui.label("Offset relativo");
                for v in [&mut p.relative_offset.x, &mut p.relative_offset.y, &mut p.relative_offset.z] {
                    let r = ui.add(egui::DragValue::new(v).speed(0.01).fixed_decimals(2));
                    acc((r.changed(), r.drag_started() || r.gained_focus()));
                }
            });
            acc(cb(ui, &mut p.merge, "Soldar"));
        }
        Modifier::Weld(p) => {
            acc(dv(ui, "Distância", &mut p.distance, 0.0005));
            acc(cb(ui, &mut p.only_loose, "Só soltos/fronteira"));
        }
        Modifier::EdgeSplit(p) => {
            let mut deg = p.angle.map(|a| a.to_degrees()).unwrap_or(30.0);
            let mut use_angle = p.angle.is_some();
            acc(cb(ui, &mut use_angle, "Por ângulo"));
            acc(dv(ui, "Ângulo (°)", &mut deg, 0.5));
            p.angle = use_angle.then_some(deg.to_radians());
            acc(cb(ui, &mut p.use_sharp, "Arestas duras"));
        }
        Modifier::Displace(p) => {
            acc(dv(ui, "Força", &mut p.strength, 0.005));
            acc(dv(ui, "Nível médio", &mut p.midlevel, 0.01));
            let noisy = matches!(p.texture, DisplaceSource::Noise { .. });
            let mut want_noise = noisy;
            acc(cb(ui, &mut want_noise, "Ruído"));
            if want_noise && !noisy {
                p.texture = DisplaceSource::Noise { scale: 1.0, seed: 1 };
            } else if !want_noise && noisy {
                p.texture = DisplaceSource::Constant;
            }
            if let DisplaceSource::Noise { scale, seed } = &mut p.texture {
                acc(dv(ui, "Escala", scale, 0.01));
                acc(du(ui, "Semente", seed, 0..=9999));
            }
        }
        Modifier::Decimate(s) => match s {
            DecimateSettings::Collapse { ratio, .. } => {
                let mut r = *ratio;
                let res = dv(ui, "Razão", &mut r, 0.005);
                *ratio = r.clamp(0.02, 1.0);
                acc(res);
            }
            DecimateSettings::Planar { angle } => {
                let mut deg = angle.to_degrees();
                acc(dv(ui, "Ângulo (°)", &mut deg, 0.5));
                *angle = deg.to_radians();
            }
        },
        Modifier::Triangulate(_) => {
            ui.label(RichText::new("Triangula todas as faces").color(theme::TEXT_DIM).size(11.5));
        }
    }
    (changed, started)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modeling::{EvalMode, ModelingSession, PrimitiveKind};
    use glam::Vec3;

    #[test]
    fn catalog_covers_every_modifier_kind_with_unique_names() {
        let c = catalog();
        assert_eq!(c.len(), 9);
        let mut names: Vec<_> = c.iter().map(|(n, _)| *n).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), 9);
    }

    #[test]
    fn every_catalog_modifier_evaluates_on_a_sphere_without_error() {
        let mut s = ModelingSession::new();
        s.add_primitive(PrimitiveKind::QuadSphere, Vec3::ZERO);
        for (name, m) in catalog() {
            let o = s.active_object_mut().unwrap();
            o.modifiers.modifiers.clear();
            o.modifiers.push(ModifierEntry::new(m));
            let r = o.evaluated(EvalMode::Viewport);
            match r {
                Some(Ok(mesh)) => assert!(mesh.face_count() > 0, "{name}"),
                Some(Err(e)) => panic!("{name}: {e}"),
                None => panic!("{name}: deveria avaliar"),
            }
        }
    }

    #[test]
    fn apply_stack_bakes_into_base_with_undo() {
        let mut s = ModelingSession::new();
        s.add_primitive(PrimitiveKind::Cube, Vec3::ZERO);
        let base = s.active_object().unwrap().mesh.clone();
        s.active_object_mut().unwrap().modifiers.push(ModifierEntry::new(Modifier::Subsurf(SubsurfSettings::default())));
        s.apply_modifier_stack().unwrap();
        let o = s.active_object().unwrap();
        assert!(o.modifiers.modifiers.is_empty());
        assert_eq!(o.mesh.face_count(), 24, "subsurf nível 1 do cubo = 24 quads");
        s.undo();
        let o = s.active_object().unwrap();
        assert_eq!(o.mesh, base);
        assert_eq!(o.modifiers.modifiers.len(), 1, "undo devolve a pilha");
        assert!(s.apply_modifier_stack().is_ok());
        s.active_object_mut().unwrap().modifiers.modifiers.clear();
        assert!(s.apply_modifier_stack().is_err());
    }

    #[test]
    fn modifiers_hash_changes_with_parameters() {
        let mut s = ModelingSession::new();
        s.add_primitive(PrimitiveKind::Cube, Vec3::ZERO);
        let h0 = s.active_object().unwrap().modifiers_hash();
        s.active_object_mut().unwrap().modifiers.push(ModifierEntry::new(Modifier::Mirror(MirrorParams::default())));
        let h1 = s.active_object().unwrap().modifiers_hash();
        assert_ne!(h0, h1);
    }

    #[test]
    fn object_with_modifiers_survives_session_roundtrip() {
        let mut s = ModelingSession::new();
        s.add_primitive(PrimitiveKind::Cube, Vec3::ZERO);
        s.active_object_mut().unwrap().modifiers.push(ModifierEntry::new(Modifier::Solidify(SolidifyParams::default())));
        let json = serde_json::to_string(&s.doc).unwrap();
        let back: crate::modeling::Document = serde_json::from_str(&json).unwrap();
        assert_eq!(back.objects[0].modifiers.modifiers.len(), 1);
    }
}
