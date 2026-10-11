//! Viewport do workspace Modelagem: render (Workbench), interação (orbitar, selecionar,
//! G/R/S modal, gizmo) e overlays em egui.

use super::gizmo;
use super::{mode_label, shading_label, CachedObject, StudioApp, ViewShading};
use crate::i18n::tr;
use crate::icons;
use crate::modeling::{Axis, ModalContext, Mode, TransformKind};
use crate::theme;
use crate::viewport::camera::ViewPreset;
use crate::viewport::picking::{self, ClickHit, PickContext};
use crate::workbench::{build_draw_data, build_draw_data_eval, BuildOptions, DrawItem, ShadingMode};
use anigo_modeling::{SelectMode, Selection, Topology};
use eframe::wgpu;
use egui::{Color32, Key, Modifiers, PointerButton, Pos2, Rect, RichText, Sense, Stroke};
use glam::{Mat4, Vec2, Vec3};

fn to_glam(p: egui::Vec2) -> Vec2 {
    Vec2::new(p.x, p.y)
}

fn hash_u64(parts: &[u64]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for p in parts {
        for b in p.to_le_bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x100_0000_01b3);
        }
    }
    h
}

impl StudioApp {
    pub(crate) fn viewport_ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| {
                let avail = ui.available_size();
                let (rect, resp) = ui.allocate_exact_size(avail, Sense::click_and_drag());
                self.viewport_hovered = resp.hovered();
                let ppp = ui.ctx().pixels_per_point();
                self.render_into(frame, rect, ppp);

                let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
                ui.painter().rect_filled(rect, 0.0, theme::BG_DEEP);
                if self.view_shading == ViewShading::Cel {
                    if let Some(tex) = self.gpu.as_ref().and_then(|g| g.cel_tex) {
                        ui.painter().image(tex, rect, uv, Color32::WHITE);
                    }
                }
                if let Some(tex) = self.gpu.as_ref().and_then(|g| g.tex) {
                    ui.painter().image(tex, rect, uv, Color32::WHITE);
                }

                let size = Vec2::new(rect.width(), rect.height());
                let local = |p: Pos2| to_glam(p - rect.min);
                if let Some(p) = resp.hover_pos().or(ui.input(|i| i.pointer.latest_pos())) {
                    self.last_mouse = p;
                }
                self.handle_viewport_input(ui, &resp, rect, size, ppp);
                self.draw_overlays(ui, rect, size);
                let _ = local;
            });
    }

    // ---------------------------------------------------------------- GPU sync

    fn render_into(&mut self, frame: &mut eframe::Frame, rect: Rect, ppp: f32) {
        if !self.ensure_gpu(frame) {
            return;
        }
        let size_px = crate::workbench::px_size(Vec2::new(rect.width(), rect.height()), ppp);
        let session = &self.modeling;
        let edit_obj = (session.mode == Mode::Edit).then_some(session.doc.active).flatten();
        let Some(gpu) = self.gpu.as_mut() else {
            return;
        };

        // reconstrói/atualiza buffers por objeto
        let mut new_cache: Vec<CachedObject> = Vec::with_capacity(session.doc.objects.len());
        let mut order: Vec<usize> = Vec::new();
        for (i, obj) in session.doc.objects.iter().enumerate() {
            if !obj.visible {
                continue;
            }
            let is_edit = edit_obj == Some(i);
            let is_active = session.doc.active == Some(i);
            let hover_wire = self.overlay_wire;
            let key = hash_u64(&[
                obj.mesh.content_hash(),
                obj.modifiers_hash(),
                if is_edit { session.revision } else { 0 },
                is_edit as u64,
                (is_active && !is_edit) as u64,
                hover_wire as u64,
                session.select_mode as u64,
            ]);
            let prev = gpu.cache.iter().position(|c| c.id == obj.id && c.key == key);
            let model = obj.model_matrix();
            let tint = [1.0, 1.0, 1.0, 1.0];
            if let Some(pos) = prev {
                let c = gpu.cache.swap_remove(pos);
                gpu.wb.set_object_transform(&c.mesh, model, tint);
                new_cache.push(c);
            } else {
                let opts = BuildOptions {
                        edit_mode: is_edit,
                        select_mode: session.select_mode,
                        selection: is_edit.then_some(&session.selection),
                        active_vert: session.active_vert,
                        object_wire: hover_wire,
                        highlight_object: is_active && !is_edit && std::env::var("ANIGO_NOHL").is_err(),
                        ..Default::default()
                };
                let data = match obj.evaluated(crate::modeling::EvalMode::Viewport) {
                    Some(Ok(eval)) => build_draw_data_eval(&obj.mesh, &eval, &opts),
                    Some(Err(e)) => {
                        self.shader_error = Some(format!("modificador: {e}"));
                        build_draw_data(&obj.mesh, &opts)
                    }
                    None => build_draw_data(&obj.mesh, &opts),
                };
                let mesh = gpu.wb.upload(&data, model, tint);
                new_cache.push(CachedObject { id: obj.id, key, mesh });
            }
            order.push(new_cache.len() - 1);
        }
        gpu.cache = new_cache;

        let items: Vec<DrawItem> = gpu.cache.iter().map(|c| DrawItem { mesh: &c.mesh }).collect();
        let mut settings = self.settings;
        settings.xray = self.modeling.xray;
        settings.shading = match self.view_shading {
            ViewShading::Solid => ShadingMode::Solid,
            ViewShading::Wireframe => ShadingMode::Wireframe,
            ViewShading::Cel => ShadingMode::OverlayOnly,
        };
        let recreated = gpu.wb.render(size_px, &self.camera, &items, &settings);
        drop(items);

        // preview cel-shaded (renderer canônico) por baixo dos overlays
        let mut cel_recreated = false;
        if self.view_shading == ViewShading::Cel {
            if let Some(cel) = gpu.cel.as_mut() {
                cel.sync_scene(&self.modeling.doc, &self.camera, size_px.0 as f32 / size_px.1.max(1) as f32);
                cel_recreated = cel.render(size_px);
                if let Some(e) = &cel.last_error {
                    self.shader_error = Some(format!("cel: {e}"));
                }
            }
        }

        if let Some(rs) = frame.wgpu_render_state() {
            if recreated || gpu.tex.is_none() {
                let mut renderer = rs.renderer.write();
                if let Some(old) = gpu.tex.take() {
                    renderer.free_texture(&old);
                }
                if let Some(view) = gpu.wb.output_view() {
                    gpu.tex = Some(renderer.register_native_texture(&rs.device, view, wgpu::FilterMode::Linear));
                }
            }
            if self.view_shading == ViewShading::Cel && (cel_recreated || gpu.cel_tex.is_none()) {
                let mut renderer = rs.renderer.write();
                if let Some(old) = gpu.cel_tex.take() {
                    renderer.free_texture(&old);
                }
                if let Some(view) = gpu.cel.as_ref().and_then(|c| c.view()) {
                    gpu.cel_tex = Some(renderer.register_native_texture(&rs.device, view, wgpu::FilterMode::Linear));
                }
            }
        }
    }

    // ------------------------------------------------------------------- entrada

    fn handle_viewport_input(&mut self, ui: &mut egui::Ui, resp: &egui::Response, rect: Rect, size: Vec2, _ppp: f32) {
        let aspect = size.x / size.y.max(1.0);
        let mouse_local = to_glam(self.last_mouse - rect.min);

        // ---- modal ativo tem prioridade total
        if self.modeling.modal_active() {
            self.handle_modal_input(ui, resp, mouse_local, size);
            return;
        }

        let (shift, ctrl, alt) = ui.input(|i| (i.modifiers.shift, i.modifiers.command, i.modifiers.alt));
        let sculpt_consumed = self.modeling.mode == Mode::Sculpt && self.sculpt_input(ui, resp, rect, size);

        // ---- câmera
        if resp.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll.abs() > 0.0 {
                let factor = (1.0 - scroll * 0.0025).clamp(0.5, 1.5);
                let ndc = Vec2::new(mouse_local.x / size.x * 2.0 - 1.0, 1.0 - mouse_local.y / size.y * 2.0);
                self.camera.zoom_to_cursor(factor, ndc, aspect);
            }
        }
        let delta = ui.input(|i| i.pointer.delta());
        let mid = resp.dragged_by(PointerButton::Middle);
        let alt_orbit = alt && !sculpt_consumed && resp.dragged_by(PointerButton::Primary);
        if mid || alt_orbit {
            if shift {
                self.camera.pan(delta.x, delta.y, size.y);
            } else if ctrl {
                self.camera.zoom(1.0 + delta.y * 0.01);
            } else {
                self.camera.orbit(delta.x, delta.y);
            }
        }

        if sculpt_consumed {
            return;
        }
        // ---- gizmo / seleção
        let pivot = self.gizmo_pivot();
        let tool_kind = self.current_transform_tool();
        if let (Some(pivot), Some(kind)) = (pivot, tool_kind) {
            if resp.drag_started_by(PointerButton::Primary) && !alt {
                if let Some(axis) = gizmo::hit_test(kind, &self.camera, size, pivot, mouse_local) {
                    if self.modeling.begin_modal(kind, mouse_local) {
                        if let Some(m) = self.modeling.modal_mut() {
                            m.toggle_axis(axis, false);
                        }
                        self.gizmo_drag = true;
                        return;
                    }
                }
            }
        }

        if resp.drag_started_by(PointerButton::Primary) && !alt {
            self.box_start = Some(mouse_local);
        }
        if resp.drag_stopped_by(PointerButton::Primary) {
            if let Some(start) = self.box_start.take() {
                let end = mouse_local;
                if (end - start).length() > 4.0 {
                    self.apply_box_select(start.min(end), start.max(end), size, shift);
                } else {
                    self.click_select(mouse_local, size, shift);
                }
            }
        } else if resp.clicked_by(PointerButton::Primary) && !alt {
            self.click_select(mouse_local, size, shift);
            self.box_start = None;
        }

        // ---- teclas (só com o ponteiro no viewport e sem foco de texto)
        if !resp.hovered() || ui.ctx().text_edit_focused() {
            return;
        }
        self.handle_hotkeys(ui, mouse_local, size);
    }

    fn handle_modal_input(&mut self, ui: &mut egui::Ui, resp: &egui::Response, mouse: Vec2, size: Vec2) {
        let ctx = ModalContext {
            camera: self.camera,
            viewport: size,
        };
        let (confirm_key, cancel_key, backspace, shift, ctrl) = ui.input(|i| {
            (
                i.key_pressed(Key::Enter),
                i.key_pressed(Key::Escape),
                i.key_pressed(Key::Backspace),
                i.modifiers.shift,
                i.modifiers.command,
            )
        });
        // eixos
        let axis_keys = [(Key::X, Axis::X), (Key::Y, Axis::Y), (Key::Z, Axis::Z)];
        let pressed: Vec<Axis> = ui.input(|i| axis_keys.iter().filter(|(k, _)| i.key_pressed(*k)).map(|(_, a)| *a).collect());
        let text: String = ui.input(|i| {
            i.events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::Text(t) => Some(t.clone()),
                    _ => None,
                })
                .collect()
        });
        if let Some(m) = self.modeling.modal_mut() {
            for a in pressed {
                m.toggle_axis(a, shift);
            }
            for ch in text.chars() {
                if ch.is_ascii_digit() || ch == '.' || ch == ',' || ch == '-' {
                    m.push_numeric(ch);
                }
            }
            if backspace {
                m.pop_numeric();
            }
            m.snap_step = if ctrl { if m.kind == TransformKind::Grab { 0.25 } else { 1.0 } } else { 0.0 };
        }
        self.modeling.update_modal(mouse, &ctx);

        // confirmar: Enter, clique esquerdo (ou soltar o botão no arrasto do gizmo)
        let released = self.gizmo_drag && ui.input(|i| i.pointer.primary_released());
        let click_confirm = !self.gizmo_drag && ui.input(|i| i.pointer.primary_pressed());
        if confirm_key || click_confirm || released {
            self.modeling.confirm_modal();
            self.gizmo_drag = false;
            self.modeling.status.clear();
        } else if cancel_key || ui.input(|i| i.pointer.secondary_pressed()) {
            self.modeling.cancel_modal();
            self.gizmo_drag = false;
            self.modeling.status.clear();
        } else if let Some(t) = self.modeling.modal_status(mouse, &ctx) {
            self.modeling.status = t;
        }
        let _ = resp;
        ui.ctx().request_repaint();
    }

    fn handle_hotkeys(&mut self, ui: &mut egui::Ui, mouse: Vec2, size: Vec2) {
        let consume = |ui: &mut egui::Ui, mods: Modifiers, key: Key| ui.input_mut(|i| i.consume_key(mods, key));
        // undo/redo
        if consume(ui, Modifiers::COMMAND | Modifiers::SHIFT, Key::Z) || consume(ui, Modifiers::COMMAND, Key::Y) {
            self.modeling.redo();
            return;
        }
        if consume(ui, Modifiers::COMMAND, Key::Z) {
            self.modeling.undo();
            return;
        }
        if consume(ui, Modifiers::NONE, Key::Tab) {
            self.modeling.toggle_edit_mode();
        }
        for (key, kind) in [(Key::G, TransformKind::Grab), (Key::R, TransformKind::Rotate), (Key::S, TransformKind::Scale)] {
            if consume(ui, Modifiers::NONE, key) {
                self.modeling.begin_modal(kind, mouse);
                return;
            }
        }
        if consume(ui, Modifiers::ALT, Key::A) {
            self.modeling.select_none();
        } else if consume(ui, Modifiers::NONE, Key::A) {
            self.modeling.select_all();
        }
        if consume(ui, Modifiers::COMMAND, Key::I) {
            self.modeling.select_invert();
        }
        if consume(ui, Modifiers::COMMAND, Key::Plus) {
            self.modeling.select_more();
        }
        if consume(ui, Modifiers::COMMAND, Key::Minus) {
            self.modeling.select_less();
        }
        if self.modeling.mode == Mode::Edit {
            for (key, m) in [(Key::Num1, SelectMode::Vertex), (Key::Num2, SelectMode::Edge), (Key::Num3, SelectMode::Face)] {
                if consume(ui, Modifiers::NONE, key) {
                    self.modeling.set_select_mode(m);
                }
            }
            if consume(ui, Modifiers::NONE, Key::L) {
                self.modeling.select_linked();
            }
            if consume(ui, Modifiers::NONE, Key::O) {
                self.modeling.proportional.enabled = !self.modeling.proportional.enabled;
            }
            self.edit_hotkeys(ui);
        } else if self.modeling.mode == Mode::Object {
            if consume(ui, Modifiers::NONE, Key::X) || consume(ui, Modifiers::NONE, Key::Delete) {
                self.modeling.delete_active_object();
            }
            if consume(ui, Modifiers::SHIFT, Key::D) {
                self.modeling.duplicate_active_object();
                self.modeling.begin_modal(TransformKind::Grab, mouse);
            }
        }
        if consume(ui, Modifiers::ALT, Key::Z) {
            self.modeling.xray = !self.modeling.xray;
        }
        if consume(ui, Modifiers::SHIFT, Key::Z) {
            self.view_shading = match self.view_shading {
                ViewShading::Solid => ViewShading::Wireframe,
                _ => ViewShading::Solid,
            };
        }
        if consume(ui, Modifiers::NONE, Key::Period) || consume(ui, Modifiers::NONE, Key::Home) {
            self.frame_selected_or_all();
        }
        let _ = size;
    }

    pub(crate) fn frame_selected_or_all(&mut self) {
        let Some(obj) = self.modeling.active_object() else {
            return;
        };
        let model = obj.model_matrix();
        let pts: Vec<Vec3> = if self.modeling.mode == Mode::Edit && !self.modeling.selection.verts.is_empty() {
            self.modeling.selection.verts.iter().map(|v| model.transform_point3(obj.mesh.co(*v))).collect()
        } else {
            obj.mesh.verts.iter().map(|v| model.transform_point3(v.co)).collect()
        };
        if let Some(first) = pts.first().copied() {
            let (lo, hi) = pts.iter().fold((first, first), |(l, h), p| (l.min(*p), h.max(*p)));
            self.camera.frame_bounds(lo, hi);
        }
    }

    fn pick_context<'a>(&'a self, size: Vec2, mesh: &'a anigo_modeling::EditMesh, camera: &'a crate::viewport::OrbitCamera) -> PickContext<'a> {
        PickContext {
            mesh,
            camera,
            size,
            xray: self.modeling.xray,
        }
    }

    fn click_select(&mut self, mouse: Vec2, size: Vec2, extend: bool) {
        match self.modeling.mode {
            Mode::Edit => {
                let Some(obj) = self.modeling.active_object() else {
                    return;
                };
                // transforma o raio para o espaço local do objeto: constrói uma câmera "local"
                // aplicando a matriz inversa nos vértices é caro; em vez disso projetamos
                // com a câmera de mundo sobre uma cópia posicionada (malha em mundo).
                let world = world_mesh(obj);
                let topo = Topology::build(&world);
                let ctx = self.pick_context(size, &world, &self.camera);
                let hit = picking::pick(&ctx, &topo, self.modeling.select_mode, mouse);
                let sel = picking::apply_click(&world, &self.modeling.selection, self.modeling.select_mode, hit, extend);
                let active = match hit {
                    ClickHit::Vertex(v) => Some(v),
                    _ => self.modeling.active_vert,
                };
                self.modeling.set_selection(sel, active);
            }
            Mode::Object => {
                let (o, d) = self.camera.ray_from_pixel(Vec2::new(mouse.x, mouse.y), size);
                let mut best: Option<(usize, f32)> = None;
                for (i, obj) in self.modeling.doc.objects.iter().enumerate() {
                    if !obj.visible {
                        continue;
                    }
                    let inv = obj.model_matrix().inverse();
                    let lo = inv.transform_point3(o);
                    let ld = inv.transform_vector3(d);
                    if let Some((_, t)) = picking::raycast_mesh(&obj.mesh, lo, ld) {
                        if best.is_none_or(|(_, bt)| t < bt) {
                            best = Some((i, t));
                        }
                    }
                }
                if let Some((i, _)) = best {
                    self.modeling.set_active(i);
                }
            }
            Mode::Sculpt => {}
        }
    }

    fn apply_box_select(&mut self, min: Vec2, max: Vec2, size: Vec2, extend: bool) {
        if self.modeling.mode != Mode::Edit {
            return;
        }
        let Some(obj) = self.modeling.active_object() else {
            return;
        };
        let world = world_mesh(obj);
        let topo = Topology::build(&world);
        let ctx = self.pick_context(size, &world, &self.camera);
        let boxed = picking::box_select(&ctx, &topo, min, max, self.modeling.select_mode);
        let sel = if extend {
            let mut s = self.modeling.selection.clone();
            s.verts.extend(boxed.verts.iter().copied());
            s.edges.extend(boxed.edges.iter().copied());
            s.faces.extend(boxed.faces.iter().copied());
            s
        } else {
            boxed
        };
        self.modeling.set_selection(sel, None);
    }

    // ------------------------------------------------------------------- ajudantes

    fn current_transform_tool(&self) -> Option<TransformKind> {
        if self.ws != crate::workspace::WorkspaceId::Modelagem || self.modeling.mode == Mode::Sculpt {
            return None;
        }
        let tools = self.shelf_tools();
        let idx = *self.tool_idx.get(&self.ws).unwrap_or(&0);
        match tools.get(idx).map(|t| t.id) {
            Some("move") => Some(TransformKind::Grab),
            Some("rotate") => Some(TransformKind::Rotate),
            Some("scale") => Some(TransformKind::Scale),
            _ => None,
        }
    }

    fn gizmo_pivot(&self) -> Option<Vec3> {
        match self.modeling.mode {
            Mode::Edit => self.modeling.selection_center_world(),
            Mode::Object => self.modeling.object_center_world(),
            Mode::Sculpt => None,
        }
    }

    // ------------------------------------------------------------------- overlays

    fn draw_overlays(&mut self, ui: &mut egui::Ui, rect: Rect, size: Vec2) {
        let painter = ui.painter_at(rect);

        // caixa de seleção
        if let (Some(start), true) = (self.box_start, ui.input(|i| i.pointer.primary_down())) {
            let a = rect.min + egui::vec2(start.x, start.y);
            let b = self.last_mouse;
            let r = Rect::from_two_pos(a, b);
            painter.rect_filled(r, 0.0, Color32::from_rgba_unmultiplied(255, 154, 46, 28));
            painter.rect_stroke(r, 0.0, Stroke::new(1.0, theme::SELECT), egui::StrokeKind::Inside);
        }

        // gizmo
        if !self.modeling.modal_active() || self.gizmo_drag {
            if let (Some(pivot), Some(kind)) = (self.gizmo_pivot(), self.current_transform_tool()) {
                self.draw_gizmo(&painter, rect, size, pivot, kind);
            }
        }

        // info
        let obj_name = self.modeling.active_object().map(|o| o.name.as_str()).unwrap_or("—");
        let persp = if self.camera.orthographic { "Ortogonal" } else { "Perspectiva" };
        painter.text(
            rect.min + egui::vec2(10.0, 44.0),
            egui::Align2::LEFT_TOP,
            format!("{persp}  |  {obj_name}"),
            egui::FontId::proportional(12.0),
            Color32::from_rgba_unmultiplied(220, 220, 230, 190),
        );
        if let Some(err) = &self.shader_error {
            painter.text(
                rect.min + egui::vec2(10.0, 62.0),
                egui::Align2::LEFT_TOP,
                format!("shader inválido (mantendo o anterior): {}", err.lines().next().unwrap_or("")),
                egui::FontId::proportional(12.0),
                Color32::from_rgb(0xff, 0x70, 0x70),
            );
        }
        if self.modeling.modal_active() && !self.modeling.status.is_empty() {
            let pos = Pos2::new(rect.center().x, rect.bottom() - 28.0);
            let text = self.modeling.status.clone();
            let galley = painter.layout_no_wrap(text, egui::FontId::proportional(13.0), Color32::WHITE);
            let bg = Rect::from_center_size(pos, galley.size() + egui::vec2(20.0, 10.0));
            painter.rect_filled(bg, 6.0, Color32::from_rgba_unmultiplied(20, 20, 26, 220));
            painter.galley(bg.center() - galley.size() * 0.5, galley, Color32::WHITE);
        }

        if self.modeling.mode == Mode::Sculpt {
            self.sculpt_overlay(ui, rect, size);
        }
        self.draw_axis_widget(ui, rect);
        self.draw_header(ui, rect);
    }

    fn draw_gizmo(&self, painter: &egui::Painter, rect: Rect, size: Vec2, pivot: Vec3, kind: TransformKind) {
        let cam = &self.camera;
        let s = gizmo::world_size(cam, pivot, size.y);
        let to_screen = |p: Vec3| -> Option<Pos2> {
            cam.project(p, size).map(|(px, _)| rect.min + egui::vec2(px.x, px.y))
        };
        let Some(origin) = to_screen(pivot) else {
            return;
        };
        let active_axis = if self.modeling.modal_active() {
            None
        } else {
            gizmo::hit_test(kind, cam, size, pivot, to_glam(self.last_mouse - rect.min))
        };
        for (axis, color) in [
            (Axis::X, Color32::from_rgb(0xe8, 0x45, 0x4b)),
            (Axis::Y, Color32::from_rgb(0x6c, 0xc0, 0x4a)),
            (Axis::Z, Color32::from_rgb(0x3d, 0x7a, 0xf0)),
        ] {
            let hot = active_axis == Some(axis);
            let c = if hot { Color32::from_rgb(0xff, 0xe0, 0x60) } else { color };
            let w = if hot { 3.5 } else { 2.5 };
            match kind {
                TransformKind::Grab | TransformKind::Scale => {
                    if let Some(end) = to_screen(pivot + axis.vector() * s) {
                        painter.line_segment([origin, end], Stroke::new(w, c));
                        if kind == TransformKind::Grab {
                            let dir = (end - origin).normalized();
                            let n = egui::vec2(-dir.y, dir.x);
                            painter.add(egui::Shape::convex_polygon(
                                vec![end + dir * 12.0, end + n * 6.0, end - n * 6.0],
                                c,
                                Stroke::NONE,
                            ));
                        } else {
                            painter.rect_filled(Rect::from_center_size(end, egui::vec2(10.0, 10.0)), 1.0, c);
                        }
                    }
                }
                TransformKind::Rotate => {
                    let pts: Vec<Pos2> = gizmo::ring_points(pivot, axis, s * 0.9)
                        .into_iter()
                        .filter_map(to_screen)
                        .collect();
                    painter.add(egui::Shape::line(pts, Stroke::new(w, c)));
                }
            }
        }
        painter.circle_filled(origin, 3.5, Color32::WHITE);
    }

    fn draw_axis_widget(&mut self, ui: &mut egui::Ui, rect: Rect) {
        let center = Pos2::new(rect.right() - 52.0, rect.top() + 82.0);
        let painter = ui.painter_at(rect);
        painter.circle_filled(center, 38.0, Color32::from_rgba_unmultiplied(20, 20, 26, 90));
        let cam = &self.camera;
        let mut axes = vec![
            (Vec3::X, "X", Color32::from_rgb(0xe8, 0x45, 0x4b), ViewPreset::Right),
            (Vec3::Y, "Y", Color32::from_rgb(0x6c, 0xc0, 0x4a), ViewPreset::Top),
            (Vec3::Z, "Z", Color32::from_rgb(0x3d, 0x7a, 0xf0), ViewPreset::Front),
            (-Vec3::X, "", Color32::from_rgb(0x7a, 0x30, 0x34), ViewPreset::Left),
            (-Vec3::Y, "", Color32::from_rgb(0x42, 0x68, 0x32), ViewPreset::Bottom),
            (-Vec3::Z, "", Color32::from_rgb(0x28, 0x4a, 0x8a), ViewPreset::Back),
        ];
        // mais distantes primeiro (desenha atrás → frente)
        axes.sort_by(|a, b| (a.0.dot(cam.forward())).total_cmp(&b.0.dot(cam.forward())).reverse());
        let mut clicked: Option<ViewPreset> = None;
        for (dir, label, color, preset) in axes {
            let screen = egui::vec2(dir.dot(cam.right()), -dir.dot(cam.up())) * 28.0;
            let p = center + screen;
            let hovered = ui.input(|i| i.pointer.hover_pos()).is_some_and(|h| h.distance(p) < 8.0);
            if !label.is_empty() {
                painter.line_segment([center, p], Stroke::new(2.0, color));
            }
            painter.circle_filled(p, if label.is_empty() { 5.0 } else { 8.0 }, if hovered { Color32::WHITE } else { color });
            if !label.is_empty() {
                painter.text(p, egui::Align2::CENTER_CENTER, label, egui::FontId::proportional(10.0), Color32::BLACK);
            }
            if hovered && ui.input(|i| i.pointer.primary_clicked()) {
                clicked = Some(preset);
            }
        }
        if let Some(p) = clicked {
            self.camera.set_preset(p);
        }
    }

    fn draw_header(&mut self, ui: &mut egui::Ui, rect: Rect) {
        let header = Rect::from_min_size(rect.min + egui::vec2(6.0, 6.0), egui::vec2(rect.width() - 12.0, 30.0));
        ui.scope_builder(egui::UiBuilder::new().max_rect(header), |ui| {
            egui::Frame::NONE
                .fill(Color32::from_rgba_unmultiplied(24, 24, 30, 215))
                .corner_radius(6.0)
                .inner_margin(egui::Margin::symmetric(6, 3))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let mut mode = self.modeling.mode;
                        egui::ComboBox::from_id_salt("mode")
                            .selected_text(mode_label(mode))
                            .width(130.0)
                            .show_ui(ui, |ui| {
                                for m in [Mode::Object, Mode::Edit, Mode::Sculpt] {
                                    ui.selectable_value(&mut mode, m, mode_label(m));
                                }
                            });
                        if mode != self.modeling.mode {
                            self.modeling.set_mode(mode);
                            self.tool_idx.insert(self.ws, 0);
                        }
                        if self.modeling.mode == Mode::Edit {
                            ui.separator();
                            for (m, icon, key) in [
                                (SelectMode::Vertex, icons::DOT_OUTLINE, "select.vertex"),
                                (SelectMode::Edge, icons::LINE_SEGMENT, "select.edge"),
                                (SelectMode::Face, icons::POLYGON, "select.face"),
                            ] {
                                if ui
                                    .selectable_label(self.modeling.select_mode == m, RichText::new(icon).size(15.0))
                                    .on_hover_text(tr(key))
                                    .clicked()
                                {
                                    self.modeling.set_select_mode(m);
                                }
                            }
                        }
                        if self.modeling.mode == Mode::Edit {
                            ui.separator();
                            ui.toggle_value(&mut self.show_uv_editor, "UV").on_hover_text("Editor de UV");
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            // espaço para o widget de eixos
                            ui.add_space(0.0);
                            let mut shading = self.view_shading;
                            egui::ComboBox::from_id_salt("shading")
                                .selected_text(shading_label(shading))
                                .width(110.0)
                                .show_ui(ui, |ui| {
                                    for m in [ViewShading::Solid, ViewShading::Wireframe, ViewShading::Cel] {
                                        ui.selectable_value(&mut shading, m, shading_label(m));
                                    }
                                });
                            self.view_shading = shading;
                            ui.toggle_value(&mut self.modeling.xray, RichText::new(icons::CUBE_TRANSPARENT).size(15.0))
                                .on_hover_text(tr("viewport.xray"));
                            ui.toggle_value(&mut self.settings.show_grid, RichText::new(icons::GRID_NINE).size(15.0))
                                .on_hover_text("Grid");
                            ui.toggle_value(&mut self.overlay_wire, RichText::new(icons::POLYGON).size(15.0))
                                .on_hover_text("Wireframe sobre o objeto");
                        });
                    });
                });
        });
    }
}

/// Cópia da malha com vértices em espaço de mundo (picking usa a câmera de mundo).
fn world_mesh(obj: &crate::modeling::ModelObject) -> anigo_modeling::EditMesh {
    let model: Mat4 = obj.model_matrix();
    let mut m = obj.mesh.clone();
    m.transform_positions(|p| model.transform_point3(p));
    m
}

#[allow(dead_code)]
fn unused(_: Selection) {}
