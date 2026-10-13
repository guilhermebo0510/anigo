//! Viewport do Iluminação: navegação (órbita, pan, zoom, enquadrar), gizmo de direção da luz (esfera de
//! direção + disco do sol arrastável), render (o cel canônico do Cenário) e overlays.

use crate::app::multiview::ViewLayout;
use super::{DragKind, LightDrag, LightPatch, StudioApp};
use crate::i18n::tr;
use crate::icons;
use crate::lighting::gizmo::{self, LineKind};
use crate::lighting::{self, ArcballHemisphere};
use crate::scene::camview::CamView;
use crate::theme;
use egui::{Color32, PointerButton, Pos2, Rect, RichText, Sense, Stroke};
use glam::{Vec2, Vec3};

/// Raio (px) da esfera de direção na tela.
const SPHERE_PX: f32 = 100.0;
/// Raio (px) de captura do disco do sol.
const DISK_HIT_PX: f32 = 15.0;

/// Altura reservada à barra de ferramentas do viewport quando há várias vistas (elas começam abaixo dela).
const HEADER_RESERVE: f32 = 44.0;

fn e2g(v: egui::Vec2) -> Vec2 {
    Vec2::new(v.x, v.y)
}

fn rgb8(c: [f32; 3]) -> Color32 {
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    Color32::from_rgb(q(c[0]), q(c[1]), q(c[2]))
}

impl StudioApp {
    pub(crate) fn light_viewport_ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| {
            let full = ui.available_rect_before_wrap();
            let ppp = ui.ctx().pixels_per_point();
            if !self.light_ui.entered {
                self.light_enter();
            }
            self.scene_refresh();
            if !self.light_ui.framed {
                self.light_frame_character();
            }
            {
                let lu = &mut self.light_ui;
                lu.mview.seed(&lu.camera);
                // o arrasto do disco do sol mantém a vista em que começou
                let busy = lu.drag.is_some();
                lu.mview.follow_pointer(&mut lu.camera, ui.ctx(), busy);
            }
            // com várias vistas a barra de ferramentas ganha uma faixa própria em vez de cobrir o topo das vistas
            let multi = self.light_ui.mview.layout != ViewLayout::Single;
            let area = if multi { Rect::from_min_max(full.min + egui::vec2(0.0, HEADER_RESERVE), full.max) } else { full };
            let slots = self.light_ui.mview.visible();
            let rects = self.light_ui.mview.pane_rects(area);
            for (&slot, &rect) in slots.iter().zip(rects.iter()) {
                self.light_ui.mview.rects[slot] = rect;
            }
            let active = self.light_ui.mview.active;
            for (&slot, &rect) in slots.iter().zip(rects.iter()) {
                if slot != active {
                    std::mem::swap(&mut self.light_ui.camera, &mut self.light_ui.mview.cams[slot]);
                }
                self.light_pane(ui, frame, slot, rect, slot == active, ppp);
                if slot != active {
                    std::mem::swap(&mut self.light_ui.camera, &mut self.light_ui.mview.cams[slot]);
                }
            }
            if slots.len() > 1 {
                ui.painter().rect_stroke(self.light_ui.mview.rects[active].shrink(0.5), 0.0, Stroke::new(1.5, theme::ACCENT_DIM), egui::StrokeKind::Inside);
                self.light_ui.mview.dividers_ui(ui, area);
            }
            let painter = ui.painter_at(full);
            self.light_toolbar(ui, full);
            self.light_hud(&painter, full);
            self.light_finish_gesture(ui.ctx());
        });
    }

    /// Uma vista da Iluminação: entrada (só a ativa), render Cel no alvo do seu slot e overlays (disco do sol).
    fn light_pane(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame, slot: usize, rect: Rect, is_active: bool, ppp: f32) {
        let resp = ui.interact(rect, ui.id().with(("light_pane", slot)), Sense::click_and_drag());
        if is_active {
            self.light_ui.view_size = Vec2::new(rect.width(), rect.height());
            self.light_input(ui, &resp, rect);
        }
        // luz presa à câmera: a direção só existe no preview (o projeto não muda) e é relativa à câmera DESTA vista
        self.scene_ui.light_dir_override = self
            .light_ui
            .follow_camera
            .then(|| lighting::camera_relative_dir(&self.light_ui.camera, self.light_ui.rel_az, self.light_ui.rel_el).to_array());
        let cam = self.light_ui.camera;
        let saved = self.scene_ui.shading;
        self.scene_ui.shading = super::super::scene_ui::SceneShading::Cel;
        self.scene_render(frame, rect, ppp, &cam, slot);
        self.scene_ui.shading = saved;

        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, theme::BG_DEEP);
        let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
        if let Some(g) = self.gpu.as_ref() {
            if let Some(t) = g.cel_tex_of(slot) {
                painter.image(t, rect, uv, Color32::WHITE);
            }
            if let Some(t) = if slot == 0 { g.tex } else { g.view_tex[slot - 1] } {
                painter.image(t, rect, uv, Color32::WHITE);
            }
            if !g.has_cel(slot) {
                painter.text(rect.center(), egui::Align2::CENTER_CENTER, tr("char.no_preview"), egui::FontId::proportional(14.0), theme::TEXT_DIM);
            }
        }
        let view = CamView::from_orbit(&cam, Vec2::new(rect.width(), rect.height()));
        self.light_overlays(&painter, rect, &view, ppp);
        if is_active {
            self.light_trace(ui.ctx(), rect, &view);
        }
        if self.light_ui.mview.layout != ViewLayout::Single {
            let area = Rect::from_min_size(rect.min + egui::vec2(8.0, 6.0), egui::vec2((rect.width() - 16.0).clamp(60.0, 330.0), 22.0));
            if let Some(k) = crate::app::multiview::view_kind_menu(ui, area, &self.light_ui.camera, "") {
                k.apply(&mut self.light_ui.camera);
            }
        }
    }

    // ===================================================================== entrada

    fn light_input(&mut self, ui: &mut egui::Ui, resp: &egui::Response, rect: Rect) {
        let ctx = ui.ctx().clone();
        let size = self.light_ui.view_size;
        let mouse_abs = resp.hover_pos().or_else(|| ctx.input(|i| i.pointer.latest_pos())).unwrap_or(rect.center());
        let mouse = e2g(mouse_abs - rect.min);
        let (shift, ctrl, alt) = ctx.input(|i| (i.modifiers.shift, i.modifiers.command, i.modifiers.alt));
        let aspect = size.x / size.y.max(1.0);

        // ---- câmera: roda = zoom; MMB / Alt+LMB = órbita; Shift+MMB = pan
        if resp.hovered() {
            let scroll = ctx.input(|i| i.smooth_scroll_delta.y);
            if scroll.abs() > 0.0 {
                let factor = (1.0 - scroll * 0.0025).clamp(0.5, 1.5);
                let ndc = Vec2::new(mouse.x / size.x * 2.0 - 1.0, 1.0 - mouse.y / size.y * 2.0);
                self.light_ui.camera.zoom_to_cursor(factor, ndc, aspect);
            }
        }
        let delta = e2g(ctx.input(|i| i.pointer.delta()));
        if self.light_ui.drag.is_none() && (resp.dragged_by(PointerButton::Middle) || (alt && resp.dragged_by(PointerButton::Primary))) {
            if shift {
                self.light_ui.camera.pan(delta.x, delta.y, size.y);
            } else if ctrl {
                self.light_ui.camera.zoom(1.0 + delta.y * 0.01);
            } else {
                self.light_ui.camera.orbit(delta.x, delta.y);
            }
        } else if alt && resp.hovered() {
            ctx.set_cursor_icon(egui::CursorIcon::Move);
        }
        if resp.hovered() && !ctx.egui_wants_keyboard_input() && self.hk(&ctx, "iluminacao.frame") {
            self.light_frame_character();
        }
        if resp.hovered() && !ctx.egui_wants_keyboard_input() {
            if self.hk(&ctx, "iluminacao.view_quad") {
                let lu = &mut self.light_ui;
                let to = if lu.mview.layout == ViewLayout::Quad { ViewLayout::Single } else { ViewLayout::Quad };
                lu.mview.set_layout(&mut lu.camera, to);
            }
            if self.hk(&ctx, "iluminacao.view_maximize") {
                let lu = &mut self.light_ui;
                lu.mview.toggle_maximize(&mut lu.camera);
            }
        }

        // ---- gizmo: disco do sol
        let view = CamView::from_orbit(&self.light_ui.camera, size);
        let center = self.light_pivot();
        let radius = lighting::gizmo_radius(view.world_per_pixel(center), SPHERE_PX);
        let dir = self.light_effective_dir();
        let sun = gizmo::sun_position(center, radius, dir);
        let disk_screen = view.project(sun).map(|(p, _)| p);
        let near_disk = |p: Vec2| disk_screen.is_some_and(|d| d.distance(p) <= DISK_HIT_PX);
        self.light_ui.hover_disk = self.light_ui.show_gizmo && resp.hovered() && near_disk(mouse);
        if self.light_ui.hover_disk || self.light_ui.drag.is_some() {
            ctx.set_cursor_icon(if self.light_ui.drag.is_some() { egui::CursorIcon::Grabbing } else { egui::CursorIcon::Grab });
        }

        if resp.drag_started_by(PointerButton::Primary) && !alt {
            let press = ctx.input(|i| i.pointer.press_origin()).map(|p| e2g(p - rect.min)).unwrap_or(mouse);
            if self.light_ui.show_gizmo && near_disk(press) {
                let facing = dir.dot((view.eye - center).normalize_or_zero());
                self.light_ui.drag = Some(LightDrag { kind: DragKind::Disk, hemi: ArcballHemisphere::starting_at(facing) });
            } else if ctrl {
                self.light_ui.drag = Some(LightDrag { kind: DragKind::Relative, hemi: ArcballHemisphere::starting_at(1.0) });
            }
        }
        if let Some(mut drag) = self.light_ui.drag.take() {
            let still = resp.dragged_by(PointerButton::Primary) && ctx.input(|i| i.pointer.primary_down());
            if still {
                match drag.kind {
                    DragKind::Disk => {
                        let (o, d) = view.ray(mouse);
                        let mut pick = lighting::pick_on_sphere(o, d, center, radius, drag.hemi.far);
                        let far = drag.hemi.step(pick.outside);
                        if far != drag.hemi.far || pick.outside {
                            pick = lighting::pick_on_sphere(o, d, center, radius, far);
                        }
                        self.light_set_direction(pick.dir, shift);
                    }
                    DragKind::Relative => {
                        let (az, el) = self.light_current_angles();
                        let (mut naz, mut nel) = (az + delta.x * 0.35, (el - delta.y * 0.35).clamp(-89.0, 89.0));
                        if shift {
                            (naz, nel) = lighting::snap_az_el(naz, nel, 15.0);
                        }
                        if self.light_ui.follow_camera {
                            // relativo à câmera: os ângulos são os da câmera, não os do mundo
                            self.light_ui.rel_az = naz;
                            self.light_ui.rel_el = nel;
                        } else {
                            self.light_set_direction(lighting::dir_from_az_el(naz, nel), false);
                        }
                    }
                }
                self.light_ui.drag = Some(drag);
                ctx.request_repaint();
            }
        }
    }

    /// Azimute/elevação atuais do sol (relativos à câmera no modo de luz presa; do mundo no resto).
    pub(crate) fn light_current_angles(&self) -> (f32, f32) {
        if self.light_ui.follow_camera {
            (self.light_ui.rel_az, self.light_ui.rel_el)
        } else {
            let d = Vec3::from_array(self.light_view().light.direction);
            lighting::az_el_from_dir(d, 0.0)
        }
    }

    /// Define a direção do sol a partir de uma direção de MUNDO (disco do gizmo). No modo de luz presa à
    /// câmera só muda o estado de preview (azimute/elevação relativos), sem comando.
    pub(crate) fn light_set_direction(&mut self, dir: Vec3, snap: bool) {
        let d = dir.normalize_or_zero();
        if d == Vec3::ZERO {
            return;
        }
        if self.light_ui.follow_camera {
            let (mut az, mut el) = lighting::camera_relative_angles(&self.light_ui.camera, d, self.light_ui.rel_az);
            if snap {
                (az, el) = lighting::snap_az_el(az, el, 15.0);
            }
            self.light_ui.rel_az = az;
            self.light_ui.rel_el = el;
            return;
        }
        let prev_az = self.light_current_angles().0;
        let (mut az, mut el) = lighting::az_el_from_dir(d, prev_az);
        if snap {
            (az, el) = lighting::snap_az_el(az, el, 15.0);
        }
        let out = lighting::dir_from_az_el(az, el);
        self.light_edit_patch(LightPatch { direction: Some(out.to_array()), ..Default::default() });
    }

    // ===================================================================== overlays

    fn light_overlays(&mut self, painter: &egui::Painter, rect: Rect, view: &CamView, ppp: f32) {
        if !self.light_ui.show_gizmo {
            self.light_ui.disk_px = None;
            return;
        }
        let to_screen = |p: Vec2| Pos2::new(rect.min.x + p.x, rect.min.y + p.y);
        let center = self.light_pivot();
        let radius = lighting::gizmo_radius(view.world_per_pixel(center), SPHERE_PX);
        let dir = self.light_effective_dir();
        let (az, el) = lighting::az_el_from_dir(dir, 0.0);
        let light_color = self.light_view().light.color;
        let sun_col = rgb8(light_color);
        let to_eye = (view.eye - center).normalize_or_zero();

        // ---- esfera + arcos (frente da esfera mais forte que o verso)
        for line in gizmo::sphere_lines(center, radius, az, el) {
            let (front, back, width) = match line.kind {
                LineKind::Grid => (Color32::from_rgba_unmultiplied(170, 180, 215, 70), Color32::from_rgba_unmultiplied(150, 160, 195, 22), 1.0),
                LineKind::Horizon => (Color32::from_rgba_unmultiplied(255, 190, 90, 190), Color32::from_rgba_unmultiplied(255, 190, 90, 60), 1.6),
                LineKind::AzimuthArc => (Color32::from_rgb(0xff, 0x9a, 0x2e), Color32::from_rgba_unmultiplied(0xff, 0x9a, 0x2e, 110), 2.4),
                LineKind::ElevationArc => (Color32::from_rgb(0xff, 0xe0, 0x3a), Color32::from_rgba_unmultiplied(0xff, 0xe0, 0x3a, 110), 2.4),
            };
            for w in line.points.windows(2) {
                let (Some((a, _)), Some((b, _))) = (view.project(w[0]), view.project(w[1])) else { continue };
                let mid = (w[0] + w[1]) * 0.5 - center;
                let facing = mid.dot(to_eye) >= 0.0 || matches!(line.kind, LineKind::AzimuthArc | LineKind::ElevationArc) && mid.dot(to_eye) > -0.25 * radius;
                painter.line_segment([to_screen(a), to_screen(b)], Stroke::new(width, if facing { front } else { back }));
            }
        }
        // marca "frente" do personagem (azimute 0°)
        if let Some((p, _)) = view.project(center + Vec3::Z * radius * 1.12) {
            painter.text(to_screen(p), egui::Align2::CENTER_CENTER, tr("light.front"), egui::FontId::proportional(11.0), Color32::from_rgba_unmultiplied(255, 190, 90, 210));
        }

        // ---- raio de luz (do sol ao personagem) com ponta de seta
        let (start, end, base) = gizmo::light_ray(center, radius, dir, radius * 0.12);
        if let (Some((a, _)), Some((b, _)), Some((c, _))) = (view.project(start), view.project(end), view.project(base)) {
            let (a, b, c) = (to_screen(a), to_screen(b), to_screen(c));
            painter.line_segment([a, b], Stroke::new(1.8, sun_col.gamma_multiply(0.85)));
            let v = (b - c).normalized();
            let n = egui::vec2(-v.y, v.x);
            painter.add(egui::Shape::convex_polygon(vec![b, c + n * 5.0, c - n * 5.0], sun_col, Stroke::NONE));
        }

        // ---- disco do sol
        let sun_world = gizmo::sun_position(center, radius, dir);
        if let Some((p, _)) = view.project(sun_world) {
            let p = to_screen(p);
            self.light_ui.disk_px = Some(Vec2::new(p.x * ppp, p.y * ppp));
            let hot = self.light_ui.hover_disk || self.light_ui.drag.is_some();
            let r = if hot { 12.5 } else { 10.5 };
            for i in 0..8 {
                let a = i as f32 * std::f32::consts::FRAC_PI_4;
                let d = egui::vec2(a.cos(), a.sin());
                painter.line_segment([p + d * (r + 3.0), p + d * (r + 7.0)], Stroke::new(2.0, sun_col));
            }
            painter.circle_filled(p, r, sun_col);
            painter.circle_stroke(p, r, Stroke::new(if hot { 2.4 } else { 1.6 }, if hot { Color32::WHITE } else { Color32::from_rgb(0x20, 0x20, 0x28) }));
            // rótulos dos ângulos junto ao disco
            painter.text(
                p + egui::vec2(0.0, -r - 16.0),
                egui::Align2::CENTER_CENTER,
                format!("{:.0}\u{b0} / {:.0}\u{b0}", az, el),
                egui::FontId::proportional(12.0),
                Color32::WHITE,
            );
        } else {
            self.light_ui.disk_px = None;
        }
    }

    fn light_toolbar(&mut self, ui: &mut egui::Ui, rect: Rect) {
        let bar = egui::Frame::NONE.fill(Color32::from_rgba_unmultiplied(24, 24, 30, 220)).corner_radius(6.0).inner_margin(egui::Margin::symmetric(6, 3));
        let r = Rect::from_min_size(rect.min + egui::vec2(8.0, 8.0), egui::vec2(rect.width() - 16.0, 30.0));
        let mut follow = self.light_ui.follow_camera;
        let mut bake = false;
        let mut frame_it = false;
        let mut front = false;
        ui.scope_builder(egui::UiBuilder::new().max_rect(r), |ui| {
            bar.show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.toggle_value(&mut self.light_ui.show_gizmo, RichText::new(icons::SPHERE).size(16.0)).on_hover_text(tr("light.gizmo_tip"));
                    ui.separator();
                    ui.toggle_value(&mut follow, format!("{}  {}", icons::VIDEO_CAMERA, tr("light.follow_camera"))).on_hover_text(tr("light.follow_camera_tip"));
                    if self.light_ui.follow_camera && ui.button(format!("{}  {}", icons::CHECK, tr("light.follow_apply"))).on_hover_text(tr("light.follow_apply_tip")).clicked() {
                        bake = true;
                    }
                    ui.separator();
                    if ui.button(RichText::new(icons::FRAME_CORNERS).size(16.0)).on_hover_text(format!("{}  (F)", tr("light.frame"))).clicked() {
                        frame_it = true;
                    }
                    if ui.button(RichText::new(icons::USER).size(16.0)).on_hover_text(tr("light.view_front")).clicked() {
                        front = true;
                    }
                    ui.separator();
                    {
                        let lu = &mut self.light_ui;
                        crate::app::multiview::layout_buttons(ui, &mut lu.mview, &mut lu.camera, false);
                    }
                });
            });
        });
        if follow != self.light_ui.follow_camera {
            self.light_set_follow(follow);
        }
        if bake {
            self.light_bake_follow();
        }
        if frame_it {
            self.light_frame_character();
        }
        if front {
            let c = &mut self.light_ui.camera;
            c.yaw = 0.0;
            c.pitch = 0.1;
        }
    }

    fn light_hud(&mut self, painter: &egui::Painter, rect: Rect) {
        let v = self.light_view();
        let (az, el) = self.light_current_angles();
        let rel = if self.light_ui.follow_camera { format!("{}  ", tr("light.relative")) } else { String::new() };
        let text = format!("{rel}{} {:.0}\u{b0}   {} {:.0}\u{b0}   {:.2}   {:.0} K", tr("light.az_short"), az, tr("light.el_short"), el, v.light.intensity, self.light_ui.temp_k);
        if self.light_ui.message_seen.0 != self.light_ui.message {
            self.light_ui.message_seen = (self.light_ui.message.clone(), Some(std::time::Instant::now()));
        }
        let fresh = self.light_ui.message_seen.1.is_some_and(|t| t.elapsed().as_secs_f32() < 6.0);
        let mut lines = vec![(text, Color32::WHITE)];
        if fresh && !self.light_ui.message.is_empty() {
            lines.push((self.light_ui.message.clone(), Color32::from_rgb(0xc8, 0xd0, 0xff)));
        }
        lines.push((tr("light.help").to_string(), theme::TEXT_DIM));
        let mut y = rect.bottom() - 10.0;
        for (t, col) in lines.into_iter().rev() {
            let galley = painter.layout_no_wrap(t, egui::FontId::proportional(12.0), col);
            let r = Rect::from_min_size(Pos2::new(rect.left() + 10.0, y - galley.size().y - 3.0), galley.size()).expand2(egui::vec2(6.0, 3.0));
            painter.rect_filled(r, 4.0, Color32::from_rgba_unmultiplied(20, 20, 26, 205));
            painter.galley(r.min + egui::vec2(6.0, 3.0), galley, col);
            y = r.top() - 3.0;
        }
    }

    /// `ANIGO_TRACE_LIGHT=1`: uma linha `lighttrace ...` por mudança de estado, para o smoke de xdotool
    /// (scripts/native_lighting_smoke.sh). Coordenadas em pixels físicos da janela.
    fn light_trace(&mut self, ctx: &egui::Context, rect: Rect, view: &CamView) {
        if std::env::var_os("ANIGO_TRACE_LIGHT").is_none() {
            return;
        }
        let ppp = ctx.pixels_per_point();
        let v = self.light_view();
        let d = Vec3::from_array(v.light.direction);
        let (az, el) = lighting::az_el_from_dir(d, 0.0);
        let h = self.core.history();
        let disk = self.light_ui.disk_px.map(|p| format!("{:.0},{:.0}", p.x, p.y)).unwrap_or_else(|| "-".into());
        let rigs: Vec<String> = self.light_ui.rig_px.iter().map(|(id, p)| format!("{id}:{:.0},{:.0}", p.x, p.y)).collect();
        let rim = v.material.as_ref().map(|m| m.rim_intensity).unwrap_or(0.0);
        let line = format!(
            "lighttrace az={az:.1} el={el:.1} int={:.2} amb={:.2} color={:.2},{:.2},{:.2} undo={} redo={} disk={disk} drag={} follow={} shadow={} rim={rim:.2} rig={} rigs={} view={:.0},{:.0},{:.0},{:.0} script={}",
            v.light.intensity,
            v.light.ambient_intensity,
            v.light.color[0],
            v.light.color[1],
            v.light.color[2],
            h.undo_depth(),
            h.redo_depth(),
            self.light_ui.drag.is_some(),
            self.light_ui.follow_camera,
            v.light.shadow.enabled,
            self.light_ui.last_rig.unwrap_or("-"),
            if rigs.is_empty() { "-".to_string() } else { rigs.join(";") },
            rect.min.x * ppp,
            rect.min.y * ppp,
            rect.max.x * ppp,
            rect.max.y * ppp,
            match &self.light_ui.script {
                None => "none",
                Some(s) if s.is_done() => "done",
                Some(_) => "running",
            },
        );
        let _ = view;
        if self.light_ui.trace.as_deref() != Some(line.as_str()) {
            eprintln!("{line}");
            self.light_ui.trace = Some(line);
        }
    }
}
