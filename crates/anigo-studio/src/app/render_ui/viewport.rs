//! Preview pela CÂMERA DE RENDER: a imagem (renderer cel canônico, mesma câmera/lente/near-far/passes/DoF do render final) com
//! máscara de enquadramento, margens seguras e guias; navegação (órbita / pan / dolly) que edita a câmera do projeto
//! com UM passo de desfazer por gesto.

use super::{CameraNav, StudioApp};
use crate::celview::LevelNodeIn;
use crate::i18n::tr;
use crate::render_out::framing;
use crate::scene::runtime::GeomState;
use crate::theme;
use crate::workbench::px_size;
use anigo_core::command::Command;
use eframe::wgpu;
use egui::{Color32, Pos2, Rect, RichText, Sense, Stroke};
use glam::Vec3;

fn cam_cmd(eye: Vec3, target: Vec3) -> Command {
    Command::SetCamera { eye: Some(eye.to_array()), target: Some(target.to_array()), up: Some([0.0, 1.0, 0.0]), fov_degrees: None, projection: None }
}

impl StudioApp {
    pub(crate) fn render_viewport_ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| {
            let avail = ui.available_size();
            let (rect, resp) = ui.allocate_exact_size(avail, Sense::click_and_drag());
            let ppp = ui.ctx().pixels_per_point();
            self.scene_refresh();
            let out = self.core.project().render.output;
            let (x, y, w, h) = framing::fit_rect((rect.min.x, rect.min.y, rect.width(), rect.height()), out.aspect(), 26.0);
            let img = Rect::from_min_size(Pos2::new(x, y), egui::vec2(w, h));
            self.render_ui.img_rect = img;
            self.render_preview_draw(frame, img, ppp);
            self.render_nav(ui.ctx(), &resp, img);

            let painter = ui.painter_at(rect);
            painter.rect_filled(rect, 0.0, theme::BG_DEEP);
            let project = self.core.project();
            if out.transparent {
                paint_checker(&painter, img);
            } else {
                let b = project.render.background_color;
                painter.rect_filled(img, 0.0, Color32::from_rgb((b[0] * 255.0) as u8, (b[1] * 255.0) as u8, (b[2] * 255.0) as u8));
            }
            if let Some(g) = self.gpu.as_ref() {
                if let Some(t) = g.cel_tex {
                    painter.image(t, img, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
                } else if g.cel.is_none() {
                    painter.text(rect.center(), egui::Align2::CENTER_CENTER, tr("char.no_preview"), egui::FontId::proportional(14.0), theme::TEXT_DIM);
                }
            }
            self.render_overlays(&painter, rect, img);
            self.render_finish_gesture(ui.ctx());
            self.render_trace(ui.ctx());

            // mensagem de status (some sozinha)
            if self.render_ui.message_seen.0 != self.render_ui.message {
                self.render_ui.message_seen = (self.render_ui.message.clone(), Some(std::time::Instant::now()));
            }
            let fresh = self.render_ui.message_seen.1.is_some_and(|t| t.elapsed().as_secs_f32() < 6.0);
            if fresh && !self.render_ui.message.is_empty() {
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
                let galley = painter.layout_no_wrap(self.render_ui.message.clone(), egui::FontId::proportional(12.5), Color32::WHITE);
                let r = Rect::from_min_size(rect.left_bottom() + egui::vec2(10.0, -10.0 - galley.size().y), galley.size()).expand2(egui::vec2(6.0, 3.0));
                painter.rect_filled(r, 4.0, Color32::from_rgba_unmultiplied(20, 20, 26, 205));
                painter.galley(r.min + egui::vec2(6.0, 3.0), galley, Color32::WHITE);
            }
        });
    }

    /// Renderiza o preview na textura do `CelPreview` (câmera exata do projeto, passes e DoF aplicados e depois limpos).
    fn render_preview_draw(&mut self, frame: &mut eframe::Frame, rect: Rect, ppp: f32) {
        if !self.ensure_gpu(frame) {
            return;
        }
        let size_px = px_size(glam::Vec2::new(rect.width(), rect.height()), ppp);
        let aspect = size_px.0 as f32 / size_px.1.max(1) as f32;
        let graph = self.render_graph();
        let project = self.core.project();
        let su = &self.scene_ui;
        let Some(gpu) = self.gpu.as_mut() else { return };
        let Some(cel) = gpu.cel.as_mut() else { return };

        let mut nodes: Vec<LevelNodeIn> = Vec::with_capacity(project.scene.nodes.len());
        for slot in &project.scene.nodes {
            let geom = match su.runtime.geom.get(&slot.node_id) {
                Some(GeomState::Ready(g)) => Some((g.key.clone(), g.mesh.clone())),
                _ => None,
            };
            nodes.push(LevelNodeIn {
                id: slot.node_id.to_string(),
                parent: slot.parent_id.as_ref().map(|p| p.to_string()),
                transform: slot.transform,
                visible: !su.runtime.hidden.contains(&slot.node_id),
                geom,
                material: super::super::scene_ui::node_material(project, slot),
            });
        }
        let light = project.scene.lights.first().map(|l| l.light.clone()).unwrap_or_default();
        let mut cam = project.scene.camera.camera.clone();
        cam.aspect = aspect;
        let orbit = super::super::scene_ui::orbit_from_camera(cam.eye, cam.target, cam.fov_y);
        cel.set_extra_lights(project.render_lights());
        let mut dirty = cel.sync_level(&nodes, light, &orbit, aspect, [0.0, 0.0, 0.0, 0.0], 0x5245_4e44);
        let dof = project.render.output.dof;
        dirty |= cel.apply_render_view(cam, graph, dof);
        let recreated = cel.render_if(size_px, dirty);
        if let Some(e) = &cel.last_error {
            self.shader_error = Some(format!("cel: {e}"));
        }
        if let Some(rs) = frame.wgpu_render_state() {
            if recreated || gpu.cel_tex.is_none() {
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

    /// Órbita (LMB), pan (MMB / Shift+LMB / RMB) e dolly (roda) da câmera de render.
    fn render_nav(&mut self, ctx: &egui::Context, resp: &egui::Response, img: Rect) {
        let cam = self.core.project().scene.camera.camera.clone();
        // ---- arrasto
        if resp.dragged() {
            let delta = resp.drag_delta();
            if delta != egui::Vec2::ZERO {
                let (shift, mid, sec) = ctx.input(|i| (i.modifiers.shift, i.pointer.middle_down(), i.pointer.secondary_down()));
                let nav = self.render_ui.nav.get_or_insert(CameraNav { eye: cam.eye, target: cam.target, last: None });
                let dist = (nav.eye - nav.target).length().max(0.05);
                if mid || sec || shift {
                    let wpp = 2.0 * dist * (cam.fov_y * 0.5).tan() / img.height().max(1.0);
                    let (e, t) = framing::pan(nav.eye, nav.target, -delta.x * wpp, delta.y * wpp);
                    nav.eye = e;
                    nav.target = t;
                } else {
                    nav.eye = framing::orbit(nav.eye, nav.target, -delta.x * 0.005, delta.y * 0.005);
                }
                let cmd = cam_cmd(nav.eye, nav.target);
                nav.last = Some(cmd.clone());
                let _ = self.core.preview(cmd);
            }
        }
        if resp.drag_stopped() {
            if let Some(nav) = self.render_ui.nav.take() {
                if let Some(cmd) = nav.last {
                    self.commit_cam(cmd);
                }
            }
        }
        // ---- roda: dolly acumulado; uma pausa fecha o passo de desfazer
        if resp.hovered() {
            let scroll: f32 = ctx.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                let w = self.render_ui.wheel.take();
                let mut nav = w.map(|(n, _)| n).unwrap_or(CameraNav { eye: cam.eye, target: cam.target, last: None });
                nav.eye = framing::dolly(nav.eye, nav.target, (1.0 - scroll * 0.0016).clamp(0.5, 2.0));
                let cmd = cam_cmd(nav.eye, nav.target);
                nav.last = Some(cmd.clone());
                let _ = self.core.preview(cmd);
                self.render_ui.wheel = Some((nav, std::time::Instant::now()));
            }
        }
        if let Some((_, t)) = &self.render_ui.wheel {
            if t.elapsed().as_secs_f32() > 0.45 && !resp.dragged() {
                if let Some((nav, _)) = self.render_ui.wheel.take() {
                    if let Some(cmd) = nav.last {
                        self.commit_cam(cmd);
                    }
                }
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(120));
            }
        }
    }

    fn commit_cam(&mut self, cmd: Command) {
        match self.core.commit_preview(cmd) {
            Ok(o) => self.render_ui.message = super::super::scene_ui::describe_scene(&o.description),
            Err(e) => {
                if !e.contains("no effect") && !e.contains("already") {
                    self.render_ui.message = e;
                }
            }
        }
    }

    fn render_overlays(&mut self, painter: &egui::Painter, rect: Rect, img: Rect) {
        let g = self.render_ui.prefs.guides;
        let out = self.core.project().render.output;
        if g.dim_outside {
            let dim = Color32::from_black_alpha(150);
            for r in [
                Rect::from_min_max(rect.min, Pos2::new(rect.max.x, img.min.y)),
                Rect::from_min_max(Pos2::new(rect.min.x, img.max.y), rect.max),
                Rect::from_min_max(Pos2::new(rect.min.x, img.min.y), Pos2::new(img.min.x, img.max.y)),
                Rect::from_min_max(Pos2::new(img.max.x, img.min.y), Pos2::new(rect.max.x, img.max.y)),
            ] {
                if r.width() > 0.5 && r.height() > 0.5 {
                    painter.rect_filled(r, 0.0, dim);
                }
            }
        }
        painter.rect_stroke(img, 0.0, Stroke::new(1.5, Color32::from_rgba_unmultiplied(255, 255, 255, 210)), egui::StrokeKind::Outside);
        let line = Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 255, 255, 110));
        if g.thirds {
            for k in [1.0 / 3.0, 2.0 / 3.0] {
                let (gx, gy) = (img.min.x + img.width() * k, img.min.y + img.height() * k);
                painter.line_segment([Pos2::new(gx, img.min.y), Pos2::new(gx, img.max.y)], line);
                painter.line_segment([Pos2::new(img.min.x, gy), Pos2::new(img.max.x, gy)], line);
            }
        }
        if g.center {
            let c = img.center();
            painter.line_segment([Pos2::new(c.x, img.min.y), Pos2::new(c.x, img.max.y)], line);
            painter.line_segment([Pos2::new(img.min.x, c.y), Pos2::new(img.max.x, c.y)], line);
            painter.circle_stroke(c, 5.0, line);
        }
        let amber = Color32::from_rgba_unmultiplied(255, 220, 90, 170);
        for (on, k, label) in [(g.action_safe, 0.9, tr("render.guide.action")), (g.title_safe, 0.8, tr("render.guide.title"))] {
            if on {
                let s = Rect::from_center_size(img.center(), img.size() * k);
                painter.rect_stroke(s, 0.0, Stroke::new(1.0, amber), egui::StrokeKind::Inside);
                painter.text(s.left_top() + egui::vec2(4.0, 2.0), egui::Align2::LEFT_TOP, label, egui::FontId::proportional(10.0), amber);
            }
        }
        if g.info {
            let c = &self.core.project().scene.camera.camera;
            let (tw, th) = out.target_size();
            let plan = crate::render_out::settings::plan(&out, 8192, crate::render_out::settings::DEFAULT_MAX_PIXELS);
            let ss = plan.as_ref().map(|p| p.ss).unwrap_or(out.supersample);
            let sensor = crate::render_out::framing::SENSORS[self.render_ui.prefs.sensor.min(crate::render_out::framing::SENSORS.len() - 1)];
            let text = format!(
                "{}\u{d7}{}  {}  \u{b7}  {:.0} mm / {:.1}\u{b0}  \u{b7}  SS {}\u{d7}{}",
                tw,
                th,
                crate::render_out::settings::aspect_label(out.width, out.height),
                crate::render_out::framing::focal_from_fov(c.fov_y, sensor.2),
                c.fov_y.to_degrees(),
                ss,
                if out.transparent { format!("  \u{b7}  {}", tr("render.info.alpha")) } else { String::new() }
            );
            let galley = painter.layout_no_wrap(text, egui::FontId::proportional(11.5), Color32::WHITE);
            let r = Rect::from_min_size(img.left_top() + egui::vec2(8.0, 6.0), galley.size()).expand2(egui::vec2(5.0, 2.0));
            painter.rect_filled(r, 3.0, Color32::from_rgba_unmultiplied(15, 15, 20, 190));
            painter.galley(r.min + egui::vec2(5.0, 2.0), galley, Color32::WHITE);
        }
        painter.text(
            Pos2::new(rect.center().x, rect.max.y - 8.0),
            egui::Align2::CENTER_BOTTOM,
            tr("render.nav.hint"),
            egui::FontId::proportional(11.0),
            theme::TEXT_DIM,
        );
        let _ = RichText::new("");
    }
}

/// Xadrez de transparência sob a imagem.
fn paint_checker(painter: &egui::Painter, r: Rect) {
    let s = 14.0;
    painter.rect_filled(r, 0.0, Color32::from_gray(58));
    let (cols, rows) = ((r.width() / s).ceil() as i32, (r.height() / s).ceil() as i32);
    for j in 0..rows {
        for i in 0..cols {
            if (i + j) % 2 == 0 {
                continue;
            }
            let min = Pos2::new(r.min.x + i as f32 * s, r.min.y + j as f32 * s);
            let sq = Rect::from_min_max(min, Pos2::new((min.x + s).min(r.max.x), (min.y + s).min(r.max.y)));
            painter.rect_filled(sq, 0.0, Color32::from_gray(84));
        }
    }
}
