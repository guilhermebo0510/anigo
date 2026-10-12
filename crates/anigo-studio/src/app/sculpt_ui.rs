//! Escultura no viewport (Modo Escultura). Hooks chamados por `inspector.rs` e `viewport_ui.rs`.
//!
//! A lógica (estado, traço, undo, ferramentas) mora em `modeling::sculpt`; aqui ficam só a tradução da
//! entrada do egui, o painel do Inspector e os overlays (círculo do pincel, simetria).

use super::StudioApp;
use crate::i18n::tr;
use crate::icons;
use crate::modeling::sculpt::{
    brush_for_tool_index, brush_tool_id, px_to_local_radius, ring_points_world, tool_index_for_brush, Resize, ScriptedStroke,
    StrokeMods, ViewInfo, RADIUS_MAX_PX, RADIUS_MIN_PX, SCULPT_BRUSHES, STRENGTH_MAX,
};
use crate::theme;
use anigo_modeling::sculpt::brush::{BrushKind, Dab};
use anigo_modeling::sculpt::falloff::{CurveProfile, Falloff};
use egui::{Color32, Key, Modifiers, Pos2, Rect, RichText, Stroke};
use glam::{Vec2, Vec3};

fn gv(p: egui::Vec2) -> Vec2 {
    Vec2::new(p.x, p.y)
}

fn section(ui: &mut egui::Ui, title: &str, open: bool, add: impl FnOnce(&mut egui::Ui)) {
    egui::CollapsingHeader::new(RichText::new(title).strong())
        .id_salt(("sculpt_section", title))
        .default_open(open)
        .show(ui, add);
}

fn falloff_label(f: &Falloff) -> &'static str {
    match f {
        Falloff::Smooth => "Suave",
        Falloff::Smoother => "Mais suave",
        Falloff::Sphere => "Esfera",
        Falloff::Root => "Raiz",
        Falloff::Sharp => "Agudo",
        Falloff::Linear => "Linear",
        Falloff::Constant => "Constante",
        Falloff::InverseSquare => "Inverso do quadrado",
        Falloff::Custom(_) => "Personalizada",
    }
}

fn axis_color(a: usize) -> Color32 {
    match a {
        0 => Color32::from_rgb(0xe8, 0x45, 0x4b),
        1 => Color32::from_rgb(0x6c, 0xc0, 0x4a),
        _ => Color32::from_rgb(0x3d, 0x7a, 0xf0),
    }
}

/// Desenha a curva de falloff (peso × distância normalizada) e, se `editable`, deixa arrastar os pontos.
/// Duplo clique cria um ponto; clique secundário remove. Devolve `true` se a curva mudou.
fn falloff_plot(ui: &mut egui::Ui, falloff: &Falloff, curve: Option<&mut CurveProfile>) -> bool {
    let size = egui::vec2(ui.available_width().min(260.0), 84.0);
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4.0, theme::BG_DEEP);
    for i in 1..4 {
        let x = rect.left() + rect.width() * i as f32 / 4.0;
        painter.line_segment([Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())], Stroke::new(1.0, Color32::from_white_alpha(10)));
    }
    let to_screen = |x: f32, y: f32| Pos2::new(rect.left() + 6.0 + (rect.width() - 12.0) * x, rect.bottom() - 6.0 - (rect.height() - 12.0) * y);
    let from_screen = |p: Pos2| {
        (
            ((p.x - rect.left() - 6.0) / (rect.width() - 12.0)).clamp(0.0, 1.0),
            ((rect.bottom() - 6.0 - p.y) / (rect.height() - 12.0)).clamp(0.0, 1.0),
        )
    };
    let pts: Vec<Pos2> = (0..=64)
        .map(|i| {
            let t = i as f32 / 64.0;
            to_screen(t, falloff.eval(t))
        })
        .collect();
    painter.add(egui::Shape::line(pts, Stroke::new(2.0, theme::ACCENT)));
    let mut changed = false;
    if let Some(curve) = curve {
        let n = curve.points().len();
        let handles: Vec<Pos2> = curve.points().iter().map(|p| to_screen(p.pos.x, p.pos.y)).collect();
        let mut grabbed: Option<usize> = ui.memory(|m| m.data.get_temp::<usize>(resp.id));
        if resp.drag_started() {
            grabbed = resp
                .interact_pointer_pos()
                .and_then(|pp| handles.iter().position(|h| h.distance(pp) < 9.0));
        }
        if resp.drag_stopped() {
            grabbed = None;
        }
        if let (Some(i), true, Some(pp)) = (grabbed, resp.dragged(), resp.interact_pointer_pos()) {
            let (x, y) = from_screen(pp);
            // as pontas ficam em x = 0 e x = 1
            let x = if i == 0 { 0.0 } else if i + 1 == n { 1.0 } else { x.clamp(0.01, 0.99) };
            if curve.move_point(i, glam::Vec2::new(x, y)).is_ok() {
                changed = true;
            }
        }
        ui.memory_mut(|m| {
            if let Some(i) = grabbed {
                m.data.insert_temp(resp.id, i);
            } else {
                m.data.remove::<usize>(resp.id);
            }
        });
        if resp.double_clicked() {
            if let Some(pp) = resp.interact_pointer_pos() {
                let (x, y) = from_screen(pp);
                if curve.add_point(x, y).is_ok() {
                    changed = true;
                }
            }
        }
        if resp.secondary_clicked() {
            if let Some(pp) = resp.interact_pointer_pos() {
                if let Some(i) = handles.iter().position(|h| h.distance(pp) < 9.0) {
                    if n > 2 && i != 0 && i + 1 != n && curve.remove_point(i).is_ok() {
                        changed = true;
                    }
                }
            }
        }
        for h in curve.points().iter().map(|p| to_screen(p.pos.x, p.pos.y)) {
            painter.circle_filled(h, 4.5, Color32::WHITE);
            painter.circle_stroke(h, 4.5, Stroke::new(1.0, Color32::BLACK));
        }
    }
    changed
}

impl StudioApp {
    /// Ferramenta do shelf → pincel ativo.
    pub(crate) fn sculpt_sync_tool(&mut self) {
        let idx = *self.tool_idx.get(&self.ws).unwrap_or(&0);
        if let Some(k) = brush_for_tool_index(idx) {
            self.sculpt.active = k;
        }
    }

    fn select_brush(&mut self, kind: BrushKind) {
        if let Some(i) = tool_index_for_brush(kind) {
            self.tool_idx.insert(self.ws, i);
        }
        self.sculpt.active = kind;
    }

    // ------------------------------------------------------------------ painel

    /// Painel do Inspector no modo Escultura (pincéis, raio/força, simetria, falloff, máscara…).
    pub(crate) fn inspector_sculpt_panel(&mut self, ui: &mut egui::Ui) {
        self.sculpt_sync_tool();
        self.sculpt.sync(&self.modeling);
        if let Some(e) = self.sculpt.error.clone() {
            ui.label(RichText::new(e).color(Color32::from_rgb(0xff, 0x80, 0x80)).size(12.0));
            ui.add_space(4.0);
        }

        section(ui, "Pincéis", true, |ui| {
            let mut pick = None;
            egui::Grid::new("sculpt_brushes").spacing([4.0, 4.0]).show(ui, |ui| {
                for (i, &k) in SCULPT_BRUSHES.iter().enumerate() {
                    let icon = crate::app::inspector::MODELAGEM_SCULPT
                        .iter()
                        .find(|t| t.id == brush_tool_id(k))
                        .map(|t| t.icon)
                        .unwrap_or(icons::CIRCLE);
                    let btn = egui::Button::new(RichText::new(icon).size(20.0))
                        .min_size(egui::vec2(38.0, 34.0))
                        .selected(self.sculpt.active == k);
                    if ui.add(btn).on_hover_text(tr(brush_tool_id(k))).clicked() {
                        pick = Some(k);
                    }
                    if (i + 1) % 5 == 0 {
                        ui.end_row();
                    }
                }
            });
            if let Some(k) = pick {
                self.select_brush(k);
            }
        });

        let kind = self.sculpt.active;
        section(ui, tr(brush_tool_id(kind)), true, |ui| {
            let is_mask = kind.is_mask();
            let default_strength = anigo_modeling::sculpt::brush::Brush::new(kind).strength;
            let smax = if default_strength <= 0.1 { 0.5 } else { STRENGTH_MAX };
            let slot = self.sculpt.slot_mut(kind);
            ui.add(egui::Slider::new(&mut slot.radius_px, RADIUS_MIN_PX..=RADIUS_MAX_PX).text("Raio (px)").logarithmic(true));
            ui.add(egui::Slider::new(&mut slot.brush.strength, 0.0..=smax).text("Força"));
            let mut spacing_pct = slot.brush.spacing * 100.0;
            if ui.add(egui::Slider::new(&mut spacing_pct, 1.0..=100.0).text("Espaçamento %")).changed() {
                slot.brush.spacing = (spacing_pct / 100.0).clamp(0.01, 10.0);
            }
            ui.checkbox(&mut slot.brush.direction_invert, "Inverter (Ctrl)");
            if !is_mask {
                ui.checkbox(&mut slot.brush.use_frontface, "Só vértices voltados para a câmera");
            }
            match kind {
                BrushKind::Draw | BrushKind::Inflate => {
                    ui.checkbox(&mut slot.brush.accumulate, "Acumular entre dabs");
                }
                BrushKind::ClayStrips => {
                    ui.add(egui::Slider::new(&mut slot.brush.strip_width, 0.1..=2.0).text("Largura da tira"));
                }
                BrushKind::Layer => {
                    ui.add(egui::Slider::new(&mut slot.brush.layer_height, 0.0..=0.5).text("Altura da camada"));
                }
                BrushKind::Crease => {
                    ui.add(egui::Slider::new(&mut slot.brush.pinch_factor, 0.0..=1.0).text("Pinçar"));
                }
                BrushKind::Elastic => {
                    ui.add(egui::Slider::new(&mut slot.brush.poisson_ratio, -0.9..=0.49).text("Poisson"));
                }
                BrushKind::Clay | BrushKind::Flatten | BrushKind::Fill | BrushKind::Scrape => {
                    ui.add(egui::Slider::new(&mut slot.brush.plane_offset, -1.0..=1.0).text("Deslocar plano"));
                }
                _ => {}
            }
            ui.add(egui::Slider::new(&mut slot.brush.stabilize_radius, 0.0..=3.0).text("Estabilizador"));
            ui.add(egui::Slider::new(&mut slot.brush.stroke_smooth, 0.0..=0.95).text("Suavizar traço"));
        });

        section(ui, "Falloff", false, |ui| {
            let kind = self.sculpt.active;
            let slot = self.sculpt.slot_mut(kind);
            let mut chosen: Option<Falloff> = None;
            egui::ComboBox::from_id_salt("sculpt_falloff")
                .selected_text(falloff_label(&slot.brush.falloff))
                .show_ui(ui, |ui| {
                    for f in Falloff::presets() {
                        let sel = slot.brush.falloff == f;
                        if ui.selectable_label(sel, falloff_label(&f)).clicked() {
                            chosen = Some(f);
                        }
                    }
                    let is_custom = matches!(slot.brush.falloff, Falloff::Custom(_));
                    if ui.selectable_label(is_custom, "Personalizada").clicked() {
                        chosen = Some(slot.curve.to_falloff());
                    }
                });
            if let Some(f) = chosen {
                slot.brush.falloff = f;
            }
            let custom = matches!(slot.brush.falloff, Falloff::Custom(_));
            let falloff = slot.brush.falloff.clone();
            let changed = falloff_plot(ui, &falloff, custom.then_some(&mut slot.curve));
            if changed {
                slot.brush.falloff = slot.curve.to_falloff();
            }
            if custom {
                ui.label(RichText::new("Arraste os pontos; duplo clique cria, clique direito remove.").size(11.0).color(theme::TEXT_DIM));
            }
        });

        section(ui, "Simetria", true, |ui| {
            ui.horizontal(|ui| {
                for (i, name) in ["X", "Y", "Z"].iter().enumerate() {
                    ui.toggle_value(&mut self.sculpt.symmetry.mirror[i], RichText::new(*name).color(axis_color(i)).strong());
                }
                ui.label(RichText::new("Espelho").size(12.0).color(theme::TEXT_DIM));
            });
            ui.horizontal(|ui| {
                ui.label(RichText::new("Radial").size(12.0).color(theme::TEXT_DIM));
                for (i, name) in ["X", "Y", "Z"].iter().enumerate() {
                    ui.label(RichText::new(*name).color(axis_color(i)).strong());
                    let mut n = self.sculpt.symmetry.radial[i];
                    if ui.add(egui::DragValue::new(&mut n).range(0..=anigo_modeling::sculpt::symmetry::MAX_RADIAL).speed(0.1)).changed() {
                        self.sculpt.symmetry.radial[i] = n;
                    }
                }
            });
        });

        section(ui, "Auto-máscara", false, |ui| {
            let kind = self.sculpt.active;
            let am = &mut self.sculpt.slot_mut(kind).brush.automask;
            let mut limit_on = am.normal_limit.is_some();
            if ui.checkbox(&mut limit_on, "Limite de normal").changed() {
                am.normal_limit = limit_on.then_some(60f32.to_radians());
                if limit_on && am.normal_falloff == 0.0 {
                    am.normal_falloff = 10f32.to_radians();
                }
            }
            if let Some(l) = am.normal_limit.as_mut() {
                let mut deg = l.to_degrees();
                if ui.add(egui::Slider::new(&mut deg, 1.0..=180.0).text("Ângulo °")).changed() {
                    *l = deg.to_radians().clamp(0.01, std::f32::consts::PI);
                }
            }
            ui.checkbox(&mut am.face_set_from_stroke, "Face set do início do traço");
            let mut cav = am.cavity.is_some();
            if ui.checkbox(&mut cav, "Cavidade").changed() {
                am.cavity = cav.then(Default::default);
            }
            if let Some(c) = am.cavity.as_mut() {
                ui.add(egui::Slider::new(&mut c.strength, 0.1..=4.0).text("Sensibilidade"));
                ui.checkbox(&mut c.invert, "Só saliências");
            }
            ui.checkbox(&mut am.topology_connected, "Topologia (conectado ao início)");
            if am.topology_connected {
                let mut d_on = am.topology_distance.is_some();
                if ui.checkbox(&mut d_on, "Limitar distância").changed() {
                    am.topology_distance = d_on.then_some(1.0);
                }
                if let Some(d) = am.topology_distance.as_mut() {
                    ui.add(egui::Slider::new(d, 0.05..=10.0).logarithmic(true).text("Distância"));
                }
            }
            ui.checkbox(&mut am.boundary, "Proteger bordas abertas");
        });

        section(ui, "Máscara global", true, |ui| {
            ui.horizontal_wrapped(|ui| {
                if ui.button(format!("{}  Inverter", icons::SELECTION_INVERSE)).clicked() {
                    let r = self.sculpt.mask_invert(&mut self.modeling);
                    self.report(r.err());
                }
                if ui.button(format!("{}  Limpar", icons::SELECTION_SLASH)).clicked() {
                    let r = self.sculpt.mask_clear(&mut self.modeling);
                    self.report(r.err());
                }
            });
            ui.label(RichText::new("Vértices mascarados (escuros) ficam protegidos dos pincéis.").size(11.0).color(theme::TEXT_DIM));
        });

        section(ui, "Remalhar", true, |ui| {
            let suggested = self.modeling.active_object().map(|o| crate::modeling::sculpt::suggest_voxel_size(&o.mesh)).unwrap_or(0.05);
            ui.horizontal(|ui| {
                ui.label("Voxel");
                ui.add(
                    egui::DragValue::new(&mut self.sculpt.voxel_size)
                        .range(0.0..=5.0)
                        .speed(0.002)
                        .fixed_decimals(3)
                        .custom_formatter(move |v, _| if v <= 0.0 { format!("auto ({suggested:.3})") } else { format!("{v:.3}") }),
                );
                ui.add(egui::DragValue::new(&mut self.sculpt.voxel_smooth).range(0..=10).prefix("suavizar ").speed(0.1));
            });
            if ui.button(format!("{}  Voxel Remesh", icons::GRID_NINE)).clicked() {
                let r = self.sculpt.voxel_remesh(&mut self.modeling);
                self.report(r.err());
            }
            ui.add_space(4.0);
            ui.checkbox(&mut self.sculpt.dyntopo, "Dyntopo (topologia dinâmica)");
            if self.sculpt.dyntopo {
                ui.add(egui::Slider::new(&mut self.sculpt.dyntopo_detail_px, 3.0..=80.0).text("Detalhe (px)"));
                ui.label(RichText::new("Subdivide/colapsa sob o pincel; melhor em malhas de poucos milhares de faces.").size(11.0).color(theme::TEXT_DIM));
            }
        });

        section(ui, "Malha", true, |ui| {
            ui.horizontal(|ui| {
                ui.label("Simetrizar");
                egui::ComboBox::from_id_salt("sym_axis")
                    .selected_text(["X", "Y", "Z"][self.sculpt.symmetrize_axis.min(2)])
                    .width(44.0)
                    .show_ui(ui, |ui| {
                        for (i, n) in ["X", "Y", "Z"].iter().enumerate() {
                            ui.selectable_value(&mut self.sculpt.symmetrize_axis, i, *n);
                        }
                    });
            });
            ui.horizontal_wrapped(|ui| {
                let axis = self.sculpt.symmetrize_axis;
                let n = ["X", "Y", "Z"][axis.min(2)];
                if ui.button(format!("+{n} {} -{n}", icons::ARROW_RIGHT)).clicked() {
                    let r = self.sculpt.symmetrize(&mut self.modeling, axis, true);
                    self.report(r.err());
                }
                if ui.button(format!("-{n} {} +{n}", icons::ARROW_RIGHT)).clicked() {
                    let r = self.sculpt.symmetrize(&mut self.modeling, axis, false);
                    self.report(r.err());
                }
            });
            ui.horizontal(|ui| {
                ui.add(egui::DragValue::new(&mut self.sculpt.smooth_all_iters).range(1..=20).suffix("×"));
                if ui.button(format!("{}  Suavizar tudo", icons::DROP)).clicked() {
                    let iters = self.sculpt.smooth_all_iters;
                    let r = self.sculpt.smooth_all(&mut self.modeling, iters, 0.5);
                    self.report(r.err());
                }
            });
        });

        section(ui, "Atalhos", false, |ui| {
            for l in [
                "Arrastar: traço   ·   Ctrl: inverte   ·   Shift: Suavizar",
                "F: raio (arrastar)   ·   Shift+F: força",
                "[ ]: raio   ·   Esc: cancela o traço",
                "MMB / Alt+arrastar: orbitar   ·   Ctrl+Z: desfazer",
            ] {
                ui.label(RichText::new(l).size(11.0).color(theme::TEXT_DIM));
            }
        });
        if let Some(s) = self.sculpt.last_stroke {
            ui.label(RichText::new(format!("Último traço: {} dabs em {:.1} ms", s.dabs, s.millis)).size(11.0).color(theme::TEXT_DIM));
        }
    }

    /// Mostra um erro amigável no status e no painel.
    fn report(&mut self, err: Option<String>) {
        if let Some(e) = err {
            self.modeling.status = e.clone();
            self.sculpt.error = Some(e);
        }
    }

    // ------------------------------------------------------------------ entrada

    /// Entrada do viewport no modo Escultura (traço do pincel, atalhos). Devolve `true` se consumiu a entrada
    /// (então o viewport não orbita/seleciona com o botão primário).
    pub(crate) fn sculpt_input(&mut self, ui: &mut egui::Ui, resp: &egui::Response, rect: Rect, size: glam::Vec2) -> bool {
        self.sculpt_sync_tool();
        self.sculpt.sync(&self.modeling);
        let view = ViewInfo { camera: self.camera, size };
        let mouse = gv(self.last_mouse - rect.min);

        // traço roteirizado (cenários de screenshot)
        if self.sculpt.script.is_some() {
            for script in [self.sculpt.script.take(), self.sculpt.followup.take()].into_iter().flatten() {
                self.sculpt.run_script(&mut self.modeling, &view, &script);
                self.sculpt.hover_override = script.hover.map(|h| h * size);
            }
            ui.ctx().request_repaint();
        }

        let (alt, ctrl, shift, pressed, down, released, time) = ui.input(|i| {
            (
                i.modifiers.alt,
                i.modifiers.command,
                i.modifiers.shift,
                i.pointer.primary_pressed(),
                i.pointer.primary_down(),
                i.pointer.primary_released(),
                i.time,
            )
        });
        let hovered = resp.hovered();
        let keys_ok = hovered && !ui.ctx().egui_wants_keyboard_input();

        // ---- modal F / Shift+F (redimensionar raio/força)
        if let Some(r) = self.sculpt.resize {
            let v = r.value_at(mouse);
            let kind = self.sculpt.active;
            let (enter, esc, rmb) = ui.input(|i| (i.key_pressed(Key::Enter), i.key_pressed(Key::Escape), i.pointer.secondary_pressed()));
            if esc || rmb {
                if r.strength {
                    self.sculpt.slot_mut(kind).brush.strength = r.start_value;
                } else {
                    self.sculpt.slot_mut(kind).radius_px = r.start_value;
                }
                self.sculpt.resize = None;
            } else {
                if r.strength {
                    self.sculpt.slot_mut(kind).brush.strength = v;
                } else {
                    self.sculpt.slot_mut(kind).radius_px = v;
                }
                if enter || pressed {
                    self.sculpt.resize = None;
                }
            }
            self.sculpt.press_captured = down;
            ui.ctx().request_repaint();
            return true;
        }

        // ---- traço em andamento
        if self.sculpt.is_stroking() {
            if ui.input(|i| i.key_pressed(Key::Escape)) {
                self.sculpt.cancel_stroke(&mut self.modeling);
                self.modeling.status = "Traço cancelado".into();
                return true;
            }
            let samples = self.pointer_samples(ui, rect);
            let pressure = touch_pressure(ui);
            for p in samples {
                self.sculpt.continue_stroke(&mut self.modeling, view.ray(p), pressure, time);
            }
            if down && !released {
                // mantém o círculo sob o cursor durante o traço
                self.sculpt.update_hover(&self.modeling, view.ray(mouse));
            } else {
                // solta: um último ponto na posição final para fechar o traço
                self.sculpt.continue_stroke(&mut self.modeling, view.ray(mouse), pressure, time);
                self.sculpt.end_stroke(&mut self.modeling);
                self.sculpt.update_hover(&self.modeling, view.ray(mouse));
                self.sculpt.press_captured = false;
            }
            ui.ctx().request_repaint();
            return true;
        }

        // ---- sem traço: hover e teclas
        if self.sculpt.hover_override.is_some() && !hovered {
            // mantém o círculo dos cenários até o mouse entrar
        } else if hovered {
            self.sculpt.hover_override = None;
            self.sculpt.update_hover(&self.modeling, view.ray(mouse));
        } else {
            self.sculpt.hover = None;
        }
        if keys_ok && !alt {
            let kind = self.sculpt.active;
            if ui.input_mut(|i| i.consume_key(Modifiers::SHIFT, Key::F)) {
                let s = self.sculpt.slot(kind).brush.strength;
                self.sculpt.resize = Some(Resize::strength(mouse, s));
            } else if ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::F)) {
                let r = self.sculpt.slot(kind).radius_px;
                self.sculpt.resize = Some(Resize::radius(mouse, r));
            }
            if ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::OpenBracket)) {
                let s = self.sculpt.slot_mut(kind);
                s.radius_px = (s.radius_px / 1.12).clamp(RADIUS_MIN_PX, RADIUS_MAX_PX);
            }
            if ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::CloseBracket)) {
                let s = self.sculpt.slot_mut(kind);
                s.radius_px = (s.radius_px * 1.12).clamp(RADIUS_MIN_PX, RADIUS_MAX_PX);
            }
            if self.sculpt.resize.is_some() {
                return true;
            }
        }

        // ---- início do traço
        if pressed && hovered && !alt {
            self.sculpt.press_captured = true;
            let mods = StrokeMods { invert: ctrl, smooth: shift };
            let pressure = touch_pressure(ui);
            match self.sculpt.begin_stroke(&mut self.modeling, &view, view.ray(mouse), pressure, time, mods) {
                Ok(_) => {}
                Err(e) => {
                    self.modeling.status = e.clone();
                    self.sculpt.error = Some(e);
                }
            }
            ui.ctx().request_repaint();
            if !self.sculpt.is_stroking() && released {
                self.sculpt.press_captured = false;
            }
            return true;
        }
        if released {
            self.sculpt.press_captured = false;
        }
        // enquanto o botão primário que começou aqui continuar apertado, nada de caixa de seleção
        self.sculpt.press_captured && down && !alt
    }

    /// Posições do ponteiro neste quadro (todas as `PointerMoved` + a última), em pixels do viewport.
    fn pointer_samples(&self, ui: &egui::Ui, rect: Rect) -> Vec<Vec2> {
        let mut out: Vec<Vec2> = Vec::new();
        ui.input(|i| {
            for e in &i.events {
                if let egui::Event::PointerMoved(p) = e {
                    out.push(gv(*p - rect.min));
                }
            }
            if let Some(p) = i.pointer.latest_pos() {
                out.push(gv(p - rect.min));
            }
        });
        out.dedup();
        if out.len() > 48 {
            let step = out.len() as f32 / 48.0;
            out = (0..48).map(|k| out[(k as f32 * step) as usize]).collect();
        }
        out
    }

    // ------------------------------------------------------------------ overlay

    /// Overlays do modo Escultura (círculo do pincel, simetria, máscara…).
    pub(crate) fn sculpt_overlay(&mut self, ui: &mut egui::Ui, rect: Rect, size: glam::Vec2) {
        let painter = ui.painter_at(rect);
        let kind = self.sculpt.active;
        let slot = self.sculpt.slot(kind).clone();
        let to_screen = |p: Vec3, cam: &crate::viewport::OrbitCamera| -> Option<Pos2> {
            cam.project(p, size).map(|(px, _)| rect.min + egui::vec2(px.x, px.y))
        };
        let ring_outer = Stroke::new(1.6, Color32::from_rgba_unmultiplied(255, 255, 255, 235));
        let ring_shadow = Stroke::new(3.4, Color32::from_rgba_unmultiplied(0, 0, 0, 120));
        let draw_poly = |pts: Vec<Pos2>, stroke: Stroke, shadow: Option<Stroke>| {
            if pts.len() > 2 {
                if let Some(sh) = shadow {
                    painter.add(egui::Shape::line(pts.clone(), sh));
                }
                painter.add(egui::Shape::line(pts, stroke));
            }
        };

        // ---- indicadores de simetria (planos de espelho no espaço do objeto)
        if let (Some(obj), Some(bvh)) = (self.modeling.active_object(), self.sculpt.bvh()) {
            if let Some(bb) = bvh.bounds() {
                let model = obj.model_matrix();
                let ext = (bb.max - bb.min).max_element().max(0.1) * 0.62;
                for axis in 0..3 {
                    let radial = self.sculpt.symmetry.radial[axis] > 1;
                    if !self.sculpt.symmetry.mirror[axis] && !radial {
                        continue;
                    }
                    let (u, v) = match axis {
                        0 => (Vec3::Y, Vec3::Z),
                        1 => (Vec3::X, Vec3::Z),
                        _ => (Vec3::X, Vec3::Y),
                    };
                    let c = axis_color(axis);
                    let corners = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0), (-1.0, -1.0)];
                    let pts: Vec<Pos2> = corners
                        .iter()
                        .filter_map(|(a, b)| to_screen(model.transform_point3((u * *a + v * *b) * ext), &self.camera))
                        .collect();
                    if pts.len() == 5 {
                        painter.add(egui::Shape::convex_polygon(pts[..4].to_vec(), Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), 22), Stroke::NONE));
                        painter.add(egui::Shape::line(pts, Stroke::new(1.2, Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), 170))));
                    }
                }
            }
        }

        // ---- modal F: círculo centrado no ponto de partida
        let mouse = gv(self.last_mouse - rect.min);
        if let Some(r) = self.sculpt.resize {
            let v = r.value_at(mouse);
            let (center, radius, text) = if r.strength {
                (r.center, slot.radius_px, format!("Força {v:.2}"))
            } else {
                (r.center, v, format!("Raio {v:.0} px"))
            };
            let c = rect.min + egui::vec2(center.x, center.y);
            painter.circle_stroke(c, radius, ring_shadow);
            painter.circle_stroke(c, radius, ring_outer);
            painter.text(
                c + egui::vec2(0.0, radius + 14.0),
                egui::Align2::CENTER_CENTER,
                text,
                egui::FontId::proportional(13.0),
                Color32::WHITE,
            );
        } else if let (Some(h), Some(obj)) = (self.sculpt.hover, self.modeling.active_object()) {
            // ---- círculo do pincel sobre a superfície (anel no plano tangente, raio de mundo)
            let model = obj.model_matrix();
            let r_local = px_to_local_radius(&self.camera, size.y, h.world_point, slot.radius_px, model);
            let ring = |center: Vec3, normal: Vec3, r: f32| -> Vec<Pos2> {
                ring_points_world(model, center, normal, r, 64)
                    .into_iter()
                    .filter_map(|p| to_screen(p, &self.camera))
                    .collect()
            };
            let mask_brush = kind.is_mask();
            let tint = if mask_brush { Color32::from_rgb(0xff, 0xb0, 0x40) } else { Color32::WHITE };
            let outer = Stroke::new(1.6, Color32::from_rgba_unmultiplied(tint.r(), tint.g(), tint.b(), 240));
            draw_poly(ring(h.local_point, h.local_normal, r_local), outer, Some(ring_shadow));
            // anel interno: 50 % do raio (referência do falloff)
            draw_poly(
                ring(h.local_point, h.local_normal, r_local * 0.5),
                Stroke::new(1.0, Color32::from_rgba_unmultiplied(tint.r(), tint.g(), tint.b(), 90)),
                None,
            );
            if let Some(c) = to_screen(h.world_point, &self.camera) {
                painter.circle_filled(c, 2.5, tint);
            }
            // cópias da simetria
            if self.sculpt.symmetry.is_active() {
                let dab = Dab::new(h.local_point, h.local_normal, r_local, slot.brush.strength);
                for d in self.sculpt.symmetry.symmetric_dabs(&dab).into_iter().skip(1) {
                    draw_poly(
                        ring(d.center, d.normal, d.radius),
                        Stroke::new(1.2, Color32::from_rgba_unmultiplied(255, 170, 60, 170)),
                        None,
                    );
                }
            }
        } else if resp_hovering(ui, rect) && self.sculpt.resize.is_none() && self.sculpt.hover_override.is_none() {
            // fora da malha: círculo de tela com o raio em pixels
            painter.circle_stroke(self.last_mouse, slot.radius_px, Stroke::new(1.2, Color32::from_rgba_unmultiplied(255, 255, 255, 110)));
        }

        // ---- legenda
        let sym: String = ["X", "Y", "Z"]
            .iter()
            .enumerate()
            .filter(|(i, _)| self.sculpt.symmetry.mirror[*i])
            .map(|(_, n)| *n)
            .collect();
        let mut line = format!(
            "{}  |  raio {:.0} px  força {:.2}",
            tr(brush_tool_id(kind)),
            slot.radius_px,
            slot.brush.strength
        );
        if !sym.is_empty() {
            line.push_str(&format!("  |  simetria {sym}"));
        }
        if self.sculpt.dyntopo {
            line.push_str("  |  dyntopo");
        }
        painter.text(
            rect.min + egui::vec2(10.0, 62.0),
            egui::Align2::LEFT_TOP,
            line,
            egui::FontId::proportional(12.0),
            Color32::from_rgba_unmultiplied(255, 200, 130, 220),
        );
        if self.sculpt.resize.is_some() || self.sculpt.is_stroking() {
            ui.ctx().request_repaint();
        }
    }
}

fn resp_hovering(ui: &egui::Ui, rect: Rect) -> bool {
    ui.input(|i| i.pointer.hover_pos()).is_some_and(|p| rect.contains(p))
}

/// Pressão da caneta (eventos de toque com força), 1.0 para mouse.
fn touch_pressure(ui: &egui::Ui) -> f32 {
    ui.input(|i| {
        i.events
            .iter()
            .rev()
            .find_map(|e| match e {
                egui::Event::Touch { force: Some(f), .. } => Some(f.clamp(0.0, 1.0)),
                _ => None,
            })
            .unwrap_or(1.0)
    })
}

/// Cenários: traço roteirizado que roda no primeiro quadro do viewport com o tamanho real.
pub(crate) fn scripted(brush: BrushKind, invert: bool, paths: Vec<Vec<(f32, f32)>>, hover: Option<(f32, f32)>) -> ScriptedStroke {
    ScriptedStroke {
        brush,
        invert,
        paths: paths.into_iter().map(|p| p.into_iter().map(|(x, y)| Vec2::new(x, y)).collect()).collect(),
        hover: hover.map(|(x, y)| Vec2::new(x, y)),
    }
}
