//! Viewport do Shading: shader ball, objeto/personagem e comparação A/B desenhados pelo `CelPreview`
//! (o MESMO renderer cel do render final), luz presa à câmera (opcional) e câmera orbital.

use super::{StudioApp, Target, BACKGROUNDS};
use crate::celview::LevelNodeIn;
use crate::i18n::tr;
use crate::icons;
use crate::shading::ball::{ball_mesh, frame_layout, plan_layout, Layout, PreviewMode, Subject, Which};
use crate::theme;
use crate::workbench::px_size;
use anigo_core::{StylizedLight, StylizedMaterial, Transform};
use eframe::wgpu;
use egui::{Color32, Pos2, Rect, RichText, Sense};
use glam::{Quat, Vec3};
use std::sync::Arc;

impl StudioApp {
    /// Malha do objeto do alvo (Personagem: deformada; Cenário: geometria do nó; Modelagem: avaliada).
    fn shading_object_subject(&mut self) -> Option<Subject> {
        let t = self.shading_ui.target.clone()?;
        match &t {
            Target::Object(id) => {
                let o = self.modeling.doc.objects.iter().find(|o| o.id == *id)?;
                let key = format!("obj:{}:{:x}", o.id, o.render_hash());
                if let Some(m) = self.shading_ui.subject_cache.get(&key) {
                    return Some(Subject::new(key, m.clone()));
                }
                let eval = o.evaluated(crate::modeling::EvalMode::Viewport);
                let src = match &eval {
                    Some(Ok(m)) => m,
                    _ => &o.mesh,
                };
                let mesh = anigo_modeling::convert::to_core_mesh(src, &anigo_modeling::convert::ToCoreOptions { normals: o.normal_params() }).ok()?;
                let mesh = Arc::new(mesh);
                if self.shading_ui.subject_cache.len() > 24 {
                    self.shading_ui.subject_cache.clear();
                }
                self.shading_ui.subject_cache.insert(key.clone(), mesh.clone());
                Some(Subject::new(key, mesh))
            }
            Target::Project(_) => {
                self.scene_refresh();
                let g = self.scene_ui.char_geom.as_ref()?;
                Some(Subject::new(g.1.key.clone(), g.1.mesh.clone()))
            }
            Target::Node(id) => {
                self.scene_refresh();
                match self.scene_ui.runtime.geom.get(id) {
                    Some(crate::scene::runtime::GeomState::Ready(g)) => Some(Subject::new(g.key.clone(), g.mesh.clone())),
                    _ => None,
                }
            }
        }
    }

    /// Assuntos do preview conforme o modo.
    fn shading_subjects(&mut self) -> Vec<Subject> {
        let mut out = Vec::new();
        let mode = self.shading_ui.mode;
        if matches!(mode, PreviewMode::Ball | PreviewMode::Both) {
            out.push(Subject::new("ball", self.shading_ui.subject_cache.entry("ball".into()).or_insert_with(ball_mesh).clone()));
        }
        if matches!(mode, PreviewMode::Object | PreviewMode::Both) {
            if let Some(s) = self.shading_object_subject() {
                out.push(s);
            } else if mode == PreviewMode::Object {
                // objeto ainda carregando/sem geometria: cai na bola
                out.push(Subject::new("ball", self.shading_ui.subject_cache.entry("ball".into()).or_insert_with(ball_mesh).clone()));
            }
        }
        out
    }

    fn shading_layout(&mut self) -> (Vec<Subject>, Layout) {
        let subjects = self.shading_subjects();
        let layout = plan_layout(&subjects, self.shading_ui.compare);
        (subjects, layout)
    }

    /// Nós de render + assinatura do conteúdo (materiais, layout) para o `CelPreview`.
    pub(crate) fn shading_build_nodes(&mut self, aspect: f32) -> Option<(Vec<LevelNodeIn>, u64)> {
        let target = self.shading_ui.target.clone()?;
        let current = self.shading_read(&target)?;
        let before = if self.shading_ui.compare { Some(self.shading_ui.snapshot_a.clone().unwrap_or_else(|| current.clone())) } else { None };
        let (subjects, layout) = self.shading_layout();
        let mut sig = anigo_core::fnv1a64(&format!("{current:?}|{before:?}|{}|{:?}", self.shading_ui.bg, self.shading_ui.face_map));
        for s in &subjects {
            sig ^= anigo_core::fnv1a64(&s.key).rotate_left(11);
        }
        sig = sig.wrapping_add(layout.items.len() as u64);
        if !self.shading_ui.framed || self.shading_ui.layout_key != sig_of_layout(&subjects, self.shading_ui.compare, self.shading_ui.mode) {
            frame_layout(&mut self.shading_ui.camera, layout.bounds, aspect);
            self.shading_ui.layout_key = sig_of_layout(&subjects, self.shading_ui.compare, self.shading_ui.mode);
            self.shading_ui.framed = true;
        }
        let mut nodes = Vec::with_capacity(layout.items.len());
        for (i, it) in layout.items.iter().enumerate() {
            let s = &subjects[it.subject];
            let material: StylizedMaterial = match it.which {
                Which::Before => before.clone().unwrap_or_else(|| current.clone()),
                Which::Current => current.clone(),
            };
            nodes.push(LevelNodeIn {
                id: format!("shade{i}"),
                parent: None,
                transform: Transform { translation: it.translation, rotation: Quat::IDENTITY, scale: Vec3::splat(it.scale) },
                visible: true,
                geom: Some((s.key.clone(), s.mesh.clone())),
                material,
            });
        }
        Some((nodes, sig))
    }

    pub(crate) fn shading_viewport_ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| {
            let avail = ui.available_size();
            let (rect, resp) = ui.allocate_exact_size(avail, Sense::click_and_drag());
            let ppp = ui.ctx().pixels_per_point();
            if self.shading_ui.turntable && !resp.dragged() {
                let dt = ui.input(|i| i.stable_dt).min(0.1);
                self.shading_ui.camera.yaw += dt * 0.6;
                ui.ctx().request_repaint();
            }
            self.shading_camera_input(ui, &resp, rect);
            self.shading_render(frame, rect, ppp);
            let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
            let bg = BACKGROUNDS[(self.shading_ui.bg as usize).min(BACKGROUNDS.len() - 1)];
            ui.painter().rect_filled(rect, 0.0, Color32::from_rgb((bg[0] * 255.0) as u8, (bg[1] * 255.0) as u8, (bg[2] * 255.0) as u8));
            if let Some(t) = self.gpu.as_ref().and_then(|g| g.cel_tex) {
                ui.painter().image(t, rect, uv, Color32::WHITE);
            }
            if self.gpu.as_ref().is_some_and(|g| g.cel.is_none()) {
                ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, tr("char.no_preview"), egui::FontId::proportional(14.0), theme::TEXT_DIM);
            }
            self.shading_labels(ui, rect);
            self.shading_viewport_header(ui, rect);
        });
    }

    fn shading_camera_input(&mut self, ui: &mut egui::Ui, resp: &egui::Response, rect: Rect) {
        use egui::PointerButton;
        let size = glam::Vec2::new(rect.width(), rect.height());
        let aspect = size.x / size.y.max(1.0);
        let cam = &mut self.shading_ui.camera;
        if resp.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll.abs() > 0.0 {
                cam.zoom((1.0 - scroll * 0.0025).clamp(0.5, 1.5));
            }
        }
        let _ = aspect;
        let (shift, ctrl) = ui.input(|i| (i.modifiers.shift, i.modifiers.command));
        let delta = ui.input(|i| i.pointer.delta());
        if resp.dragged_by(PointerButton::Primary) || resp.dragged_by(PointerButton::Middle) {
            if shift {
                cam.pan(delta.x, delta.y, size.y);
            } else if ctrl {
                cam.zoom(1.0 + delta.y * 0.01);
            } else {
                cam.orbit(delta.x, delta.y);
            }
        }
    }

    /// Rótulos A / B e do assunto sobre cada cópia (projetados na tela).
    fn shading_labels(&mut self, ui: &mut egui::Ui, rect: Rect) {
        if !self.shading_ui.compare {
            return;
        }
        let aspect = rect.width() / rect.height().max(1.0);
        let (subjects, layout) = self.shading_layout();
        let vp = self.shading_ui.camera.view_proj(aspect);
        for it in &layout.items {
            let b = subjects[it.subject].bounds;
            let top = it.translation + Vec3::new(((b.0.x + b.1.x) * 0.5) * it.scale, b.1.y * it.scale, 0.0);
            let clip = vp * top.extend(1.0);
            if clip.w <= 0.0 {
                continue;
            }
            let ndc = clip.truncate() / clip.w;
            let p = Pos2::new(rect.min.x + (ndc.x * 0.5 + 0.5) * rect.width(), rect.min.y + (0.5 - ndc.y * 0.5) * rect.height() - 14.0);
            let (text, color) = match it.which {
                Which::Before => (tr("shading.ab.before"), Color32::from_rgb(0xff, 0xb4, 0x5a)),
                Which::Current => (tr("shading.ab.after"), theme::ACCENT),
            };
            ui.painter().text(p, egui::Align2::CENTER_BOTTOM, text, egui::FontId::proportional(13.0), color);
        }
    }

    fn shading_viewport_header(&mut self, ui: &mut egui::Ui, rect: Rect) {
        let header = Rect::from_min_size(rect.min + egui::vec2(6.0, 6.0), egui::vec2((rect.width() - 12.0).max(200.0), 64.0));
        ui.scope_builder(egui::UiBuilder::new().max_rect(header), |ui| {
            egui::Frame::NONE.fill(Color32::from_rgba_unmultiplied(24, 24, 30, 215)).corner_radius(6.0).inner_margin(egui::Margin::symmetric(6, 3)).show(ui, |ui| {
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        let mut changed = false;
                        for (mode, icon, key) in [(PreviewMode::Ball, icons::SPHERE, "shading.view.ball"), (PreviewMode::Object, icons::CUBE, "shading.view.object"), (PreviewMode::Both, icons::SQUARES_FOUR, "shading.view.both")] {
                            if ui.selectable_label(self.shading_ui.mode == mode, format!("{icon} {}", tr(key))).clicked() {
                                self.shading_ui.mode = mode;
                                changed = true;
                            }
                        }
                        if changed {
                            self.shading_ui.framed = false;
                        }
                        ui.separator();
                        self.shading_ab_controls(ui);
                        ui.separator();
                        for i in 0..BACKGROUNDS.len() {
                            let c = BACKGROUNDS[i];
                            let (r, resp) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), Sense::click());
                            ui.painter().rect_filled(r, 3.0, Color32::from_rgb((c[0] * 255.0) as u8, (c[1] * 255.0) as u8, (c[2] * 255.0) as u8));
                            if self.shading_ui.bg as usize == i {
                                ui.painter().rect_stroke(r, 3.0, egui::Stroke::new(2.0, theme::ACCENT), egui::StrokeKind::Outside);
                            }
                            if resp.on_hover_text(tr("shading.bg")).clicked() {
                                self.shading_ui.bg = i as u8;
                            }
                        }
                        ui.separator();
                        ui.toggle_value(&mut self.shading_ui.turntable, icons::ARROWS_CLOCKWISE).on_hover_text(tr("char.turntable"));
                        if ui.button(icons::CORNERS_OUT).on_hover_text(tr("char.frame")).clicked() {
                            self.shading_ui.framed = false;
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.toggle_value(&mut self.shading_ui.rig.follows_camera, format!("{}  {}", icons::SUN, tr("shading.light.follow"))).on_hover_text(tr("shading.light.follow_tip"));
                        ui.spacing_mut().slider_width = 110.0;
                        ui.add(egui::Slider::new(&mut self.shading_ui.rig.azimuth_deg, -180.0..=180.0).text(tr("shading.light.az")).integer().clamping(egui::SliderClamping::Always)).on_hover_text(tr("shading.light.az_tip"));
                        ui.add(egui::Slider::new(&mut self.shading_ui.rig.elevation_deg, -80.0..=89.0).text(tr("shading.light.el")).integer().clamping(egui::SliderClamping::Always));
                    });
                });
            });
        });
    }

    /// Botões do comparador A/B (no cabeçalho do viewport).
    fn shading_ab_controls(&mut self, ui: &mut egui::Ui) {
        let on = self.shading_ui.compare;
        if ui.selectable_label(on, format!("{}  A/B", icons::COLUMNS)).on_hover_text(tr("shading.ab.tip")).clicked() {
            if !on && self.shading_ui.snapshot_a.is_none() {
                self.shading_ui.snapshot_a = self.shading_ui.target.as_ref().and_then(|t| self.shading_read(t));
            }
            self.shading_ui.compare = !on;
            self.shading_ui.framed = false;
        }
        if self.shading_ui.compare {
            if ui.button(tr("shading.ab.set_a")).on_hover_text(tr("shading.ab.set_a_tip")).clicked() {
                self.shading_ui.snapshot_a = self.shading_ui.target.as_ref().and_then(|t| self.shading_read(t));
            }
            if ui.button(format!("{}  {}", icons::ARROWS_LEFT_RIGHT, tr("shading.ab.swap"))).on_hover_text(tr("shading.ab.swap_tip")).clicked() {
                if let (Some(a), Some(cur)) = (self.shading_ui.snapshot_a.clone(), self.shading_ui.target.as_ref().and_then(|t| self.shading_read(t))) {
                    self.shading_ui.snapshot_a = Some(cur);
                    self.shading_apply_now(a);
                }
            }
            if ui.button(tr("shading.ab.revert")).on_hover_text(tr("shading.ab.revert_tip")).clicked() {
                if let Some(a) = self.shading_ui.snapshot_a.clone() {
                    self.shading_apply_now(a);
                }
            }
        }
    }

    fn shading_render(&mut self, frame: &mut eframe::Frame, rect: Rect, ppp: f32) {
        if !self.ensure_gpu(frame) {
            return;
        }
        let size_px = px_size(glam::Vec2::new(rect.width(), rect.height()), ppp);
        let aspect = size_px.0 as f32 / size_px.1.max(1) as f32;
        let Some((nodes, sig)) = self.shading_build_nodes(aspect) else { return };
        let mut light: StylizedLight = self.core.project().scene.lights.first().map(|s| s.light.clone()).unwrap_or_default();
        light.direction = self.shading_ui.rig.direction(&self.shading_ui.camera).to_array();
        let bg = BACKGROUNDS[(self.shading_ui.bg as usize).min(BACKGROUNDS.len() - 1)];
        let cam = self.shading_ui.camera;
        let light_sig = light.direction.iter().fold(0u64, |h, v| h.rotate_left(5) ^ v.to_bits() as u64);
        let Some(gpu) = self.gpu.as_mut() else { return };
        let mut cel_recreated = false;
        if let Some(cel) = gpu.cel.as_mut() {
            let changed = cel.sync_level(&nodes, light, &cam, aspect, bg, sig ^ light_sig);
            cel_recreated = cel.render_if(size_px, changed);
            if let Some(e) = &cel.last_error {
                self.shader_error = Some(format!("cel: {e}"));
            }
        }
        if let Some(rs) = frame.wgpu_render_state() {
            if cel_recreated || gpu.cel_tex.is_none() {
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

    /// Mapa SDF facial no renderer do preview.
    pub(crate) fn shading_install_face_map(&mut self, map: super::FaceMap) {
        let Some(cel) = self.gpu.as_ref().and_then(|g| g.cel.as_ref()) else {
            self.shading_ui.face_map_error = Some(tr("char.no_preview").to_string());
            return;
        };
        match &map {
            super::FaceMap::None => {
                cel.clear_face_sdf();
                self.shading_ui.face_map_error = None;
            }
            super::FaceMap::Procedural => {
                let sdf = anigo_renderer::face_sdf_map::procedural_face_sdf_f32(256, 128, 0.25, 0.25, true);
                self.shading_ui.face_map_error = cel.set_face_sdf_f32(256, 128, &sdf).err();
            }
            super::FaceMap::Library(id, _) => {
                let lib = self.lib_ws.as_ref().and_then(|w| w.library().cloned());
                let r = (|| -> Result<(u32, u32, Vec<u8>), String> {
                    let lib = lib.ok_or("a Biblioteca não está aberta")?;
                    let files = lib.resolve_files(id).map_err(|e| e.to_string())?;
                    let img = image::open(&files.primary).map_err(|e| e.to_string())?;
                    let gray = img.to_luma8();
                    let (w, h) = gray.dimensions();
                    if w == 0 || h == 0 || w > 4096 || h > 4096 {
                        return Err(format!("{w}x{h}: tamanho não suportado"));
                    }
                    Ok((w, h, gray.into_raw()))
                })();
                match r {
                    Ok((w, h, raw)) => self.shading_ui.face_map_error = cel.set_face_sdf_r8(w, h, &raw).err(),
                    Err(e) => {
                        self.shading_ui.face_map_error = Some(e);
                        return;
                    }
                }
            }
        }
        if self.shading_ui.face_map_error.is_none() {
            self.shading_ui.face_map = map;
            // o mapa é global ao renderer: força um novo desenho
            self.shading_ui.layout_key = self.shading_ui.layout_key.wrapping_add(1);
        }
    }
}

fn sig_of_layout(subjects: &[Subject], compare: bool, mode: PreviewMode) -> u64 {
    let mut h = anigo_core::fnv1a64(&format!("{compare}{mode:?}"));
    for s in subjects {
        h ^= anigo_core::fnv1a64(&s.key).rotate_left(3);
    }
    h
}

#[allow(dead_code)]
fn _unused(_: RichText) {}
