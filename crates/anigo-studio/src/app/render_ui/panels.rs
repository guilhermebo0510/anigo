//! Painéis do Render: painel direito (Câmera / Passes / Saída, conforme a tool shelf), botão "Renderizar" e a
//! fila + histórico com miniaturas.

use super::{detect_ffmpeg, dir_status, ffmpeg_command, DirStatus, JobStatus, RenderTab, StudioApp};
use crate::i18n::tr;
use crate::icons;
use crate::render_out::framing;
use crate::render_out::history::HistoryEntry;
use crate::render_out::job::Stage;
use crate::render_out::settings::{self, Axis, PresetGroup, DEFAULT_MAX_PIXELS, PRESETS};
use crate::theme;
use anigo_core::command::{CameraProjectionPatch, Command};
use anigo_core::level;
use anigo_core::{AovSelection, OutputFormat, RenderOutputPatch, TonemapOperator};
use anigo_core::project::ColorSpace;
use egui::{Color32, RichText};
use glam::Vec3;

const ERR: Color32 = Color32::from_rgb(0xff, 0x70, 0x70);
const WARN: Color32 = Color32::from_rgb(0xff, 0xc8, 0x5a);
const OK: Color32 = Color32::from_rgb(0x6a, 0xcc, 0x6a);

fn dim(text: impl Into<String>) -> RichText {
    RichText::new(text).size(11.5).color(theme::TEXT_DIM)
}

fn section(ui: &mut egui::Ui, icon: &str, key: &str, open: bool, body: impl FnOnce(&mut egui::Ui)) {
    crate::app::props_tabs::header(ui, RichText::new(format!("{icon}  {}", tr(key))).strong()).default_open(open).show(ui, body);
}

fn stage_key(s: Stage) -> &'static str {
    match s {
        Stage::Prepare => "render.stage.prepare",
        Stage::Render => "render.stage.render",
        Stage::Outline => "render.stage.outline",
        Stage::Reduce => "render.stage.reduce",
        Stage::Aov => "render.stage.aov",
        Stage::Encode => "render.stage.encode",
        Stage::Write => "render.stage.write",
    }
}

fn fmt_bytes(b: u64) -> String {
    if b >= 1 << 20 {
        format!("{:.1} MiB", b as f64 / (1 << 20) as f64)
    } else {
        format!("{:.0} KiB", (b as f64 / 1024.0).max(1.0))
    }
}

impl StudioApp {
    /// Início do quadro do Render (antes dos painéis): entra no workspace, avança a fila e fixa a aba da ferramenta.
    pub(crate) fn render_frame_begin(&mut self, ctx: &egui::Context) {
        self.render_enter(ctx);
        self.render_poll(ctx);
        self.render_ui.tab = RenderTab::from_tool_index(*self.tool_idx.get(&crate::workspace::WorkspaceId::Render).unwrap_or(&0));
    }

    /// Painel Ajustes de render (corpo): botão Renderizar + abas Câmera/Passes/Saída.
    pub(crate) fn render_settings_body(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        ui.add_space(6.0);
        ui.label(RichText::new(tr(match self.render_ui.tab {
            RenderTab::Camera => "tool.camera",
            RenderTab::Passes => "tool.passes",
            RenderTab::Output => "tool.export",
        })).strong().size(14.0));
        self.render_button(ui, &ctx);
        ui.separator();
        crate::app::props_tabs::props_scroll(ui, "render_scroll", |ui| match self.render_ui.tab {
            RenderTab::Camera => self.camera_tab(ui),
            RenderTab::Passes => self.passes_tab(ui),
            RenderTab::Output => self.output_tab(ui),
        });
    }

    /// Painel Fila de render (corpo).
    pub(crate) fn render_queue_body(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.queue_panel(ui, &ctx);
    }

    // ============================================================ botão Renderizar

    fn render_button(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let out = self.core.project().render.output;
        let plan = settings::plan(&out, 8192, DEFAULT_MAX_PIXELS);
        let aovs = self.render_aovs_enabled().len();
        ui.add_space(4.0);
        let busy = self.render_ui.running();
        let btn = egui::Button::new(RichText::new(format!("{}  {}", icons::CAMERA, tr("render.btn.render"))).strong().size(14.5)).min_size(egui::vec2(ui.available_width(), 34.0)).fill(theme::ACCENT_DIM);
        let resp = ui.add_enabled(plan.is_ok(), btn).on_hover_text(tr("render.btn.render_tip"));
        self.render_ui.render_btn = resp.rect.center();
        if resp.clicked() {
            self.render_enqueue(ctx);
        }
        let (tw, th) = out.target_size();
        let mut line = format!("{tw}\u{d7}{th}  \u{b7}  {}  \u{b7}  SS {}\u{d7}", match out.format {
            OutputFormat::Png8 => "PNG 8",
            OutputFormat::Png16 => "PNG 16",
            OutputFormat::Jpeg => "JPEG",
        }, plan.as_ref().map(|p| p.ss).unwrap_or(1));
        if out.transparent {
            line.push_str(&format!("  \u{b7}  {}", tr("render.info.alpha")));
        }
        if aovs > 0 {
            line.push_str(&format!("  \u{b7}  +{aovs} AOV"));
        }
        ui.label(dim(line));
        if let Err(e) = &plan {
            ui.label(RichText::new(e).size(11.5).color(ERR));
        }
        if busy {
            ui.label(dim(tr("render.queue.busy")));
        }
    }

    // ============================================================ Câmera

    fn camera_tab(&mut self, ui: &mut egui::Ui) {
        let project = self.core.project();
        let cam = project.scene.camera.camera.clone();
        let out = project.render.output;
        let ctx = ui.ctx().clone();

        section(ui, icons::VIDEO_CAMERA, "render.camera.source", true, |ui| {
            let cams = self.render_scene_cameras();
            let current = self.render_camera_name();
            let mut pick: Option<Option<anigo_core::ids::NodeId>> = None;
            crate::app::props_tabs::combo(ui, "render_cam_source").selected_text(current).width(ui.available_width() - 8.0).show_ui(ui, |ui| {
                if ui.selectable_label(self.render_ui.linked.is_none(), tr("render.camera.project")).clicked() {
                    pick = Some(None);
                }
                for (id, name) in &cams {
                    if ui.selectable_label(self.render_ui.linked.as_ref() == Some(id), format!("{}  {name}", icons::VIDEO_CAMERA)).clicked() {
                        pick = Some(Some(id.clone()));
                    }
                }
            });
            if let Some(p) = pick {
                self.render_ui.linked = p.clone();
                if let Some(id) = p {
                    self.render_apply_scene_camera(&id);
                }
            }
            if cams.is_empty() {
                ui.label(dim(tr("render.camera.none_hint")));
            }
            if let Some(id) = self.render_ui.linked.clone() {
                if ui.button(format!("{}  {}", icons::ARROWS_CLOCKWISE, tr("render.camera.reapply"))).on_hover_text(tr("render.camera.reapply_tip")).clicked() {
                    self.render_apply_scene_camera(&id);
                }
            }
            ui.horizontal_wrapped(|ui| {
                if ui.button(format!("{}  {}", icons::CROSSHAIR, tr("render.camera.from_view"))).on_hover_text(tr("render.camera.from_view_tip")).clicked() {
                    let c = self.scene_ui.camera;
                    self.render_apply(Command::SetCamera { eye: Some(c.eye().to_array()), target: Some(c.target.to_array()), up: Some([0.0, 1.0, 0.0]), fov_degrees: Some(c.fov_y.to_degrees()), projection: None });
                }
                if ui.button(format!("{}  {}", icons::FRAME_CORNERS, tr("render.camera.frame_all"))).on_hover_text(tr("render.camera.frame_tip")).clicked() {
                    self.render_frame(false);
                }
                let has_sel = !self.scene_ui.selection.is_empty();
                if ui.add_enabled(has_sel, egui::Button::new(format!("{}  {}", icons::CUBE_FOCUS, tr("render.camera.frame_sel")))).clicked() {
                    self.render_frame(true);
                }
            });
            ui.label(dim(format!("eye {}  \u{2192}  {}", fmt3(cam.eye), fmt3(cam.target))));
        });

        section(ui, icons::APERTURE, "render.lens", true, |ui| {
            let sensors = framing::SENSORS;
            let sidx = self.render_ui.prefs.sensor.min(sensors.len() - 1);
            let sensor_h = sensors[sidx].2;
            // rótulo em cima quando o painel é estreito; o combo nunca passa da largura disponível
            crate::app::props_tabs::field_row(ui, tr("render.lens.sensor"), 64.0, |ui| {
                crate::app::props_tabs::combo(ui, "render_sensor").selected_text(sensors[sidx].0).width((ui.available_width() - 30.0).max(80.0)).show_ui(ui, |ui| {
                    for (i, s) in sensors.iter().enumerate() {
                        if ui.selectable_label(i == sidx, s.0).clicked() {
                            self.render_ui.prefs.sensor = i;
                        }
                    }
                });
            });
            let mut fov = cam.fov_y.to_degrees();
            let mut focal = framing::focal_from_fov(cam.fov_y, sensor_h);
            let mut changed_fov = None;
            ui.horizontal_wrapped(|ui| {
                ui.label(tr("render.lens.fov"));
                if ui.add(egui::DragValue::new(&mut fov).speed(0.2).range(5.0..=120.0).suffix("\u{b0}").max_decimals(1)).changed() {
                    changed_fov = Some(fov);
                }
                ui.label(tr("render.lens.focal"));
                if ui.add(egui::DragValue::new(&mut focal).speed(0.3).range(8.0..=400.0).suffix(" mm").max_decimals(1)).changed() {
                    changed_fov = Some(framing::fov_from_focal(focal, sensor_h).to_degrees());
                }
            });
            if let Some(f) = changed_fov {
                self.render_edit(Command::SetCamera { eye: None, target: None, up: None, fov_degrees: Some(f.clamp(5.0, 120.0)), projection: None });
            }
            ui.label(dim(tr("render.lens.focal_tip")));
            ui.horizontal_wrapped(|ui| {
                ui.label(tr("render.lens.projection"));
                let ortho = cam.orthographic.is_some();
                if ui.selectable_label(!ortho, tr("render.lens.perspective")).clicked() && ortho {
                    self.render_apply(Command::SetCamera { eye: None, target: None, up: None, fov_degrees: None, projection: Some(CameraProjectionPatch { orthographic: Some(false), ..Default::default() }) });
                }
                if ui.selectable_label(ortho, tr("render.lens.ortho")).clicked() && !ortho {
                    self.render_apply(Command::SetCamera { eye: None, target: None, up: None, fov_degrees: None, projection: Some(CameraProjectionPatch { orthographic: Some(true), ..Default::default() }) });
                }
            });
            let (mut near, mut far) = (cam.z_near, cam.z_far);
            ui.horizontal_wrapped(|ui| {
                ui.label(tr("render.lens.near"));
                if ui.add(egui::DragValue::new(&mut near).speed(0.01).range(0.001..=(cam.z_far - 0.01).max(0.002)).suffix(" m").max_decimals(3)).changed() {
                    self.render_edit_patch(RenderOutputPatch { camera_near: Some(near), ..Default::default() });
                }
                ui.label(tr("render.lens.far"));
                if ui.add(egui::DragValue::new(&mut far).speed(1.0).range((cam.z_near + 0.01).min(99_999.0)..=100_000.0).suffix(" m").max_decimals(1)).changed() {
                    self.render_edit_patch(RenderOutputPatch { camera_far: Some(far), ..Default::default() });
                }
            });
            ui.label(dim(format!("{}  {}\u{d7}{}  ({})", tr("render.lens.aspect"), out.width, out.height, settings::aspect_label(out.width, out.height))));
        });

        section(ui, icons::GRID_FOUR, "render.guides", true, |ui| {
            let g = &mut self.render_ui.prefs.guides;
            ui.checkbox(&mut g.thirds, tr("render.guide.thirds"));
            ui.checkbox(&mut g.center, tr("render.guide.center"));
            ui.checkbox(&mut g.action_safe, tr("render.guide.action_safe"));
            ui.checkbox(&mut g.title_safe, tr("render.guide.title_safe"));
            ui.checkbox(&mut g.dim_outside, tr("render.guide.dim"));
            ui.checkbox(&mut g.info, tr("render.guide.info"));
        });
        let _ = ctx;
    }

    /// Aplica a câmera de cena `id` como câmera de render (UM passo de desfazer).
    pub(crate) fn render_apply_scene_camera(&mut self, id: &anigo_core::ids::NodeId) {
        match level::camera_command_from_node(self.core.project(), id) {
            Ok(c) => {
                self.render_apply(c);
            }
            Err(e) => self.render_say(e.to_string()),
        }
    }

    /// Enquadra a cena (ou a seleção) respeitando o aspecto da SAÍDA e mantendo a direção de olhar.
    pub(crate) fn render_frame(&mut self, selection: bool) {
        let cam = self.core.project().scene.camera.camera.clone();
        let aspect = self.core.project().render.output.aspect();
        let su = &self.scene_ui;
        let bounds = if selection {
            su.selection.ids().iter().filter_map(|i| su.runtime.bounds_of(i)).reduce(|a, b| a.union(&b))
        } else {
            su.runtime.bounds
        };
        let Some(b) = bounds else {
            self.render_say(tr("render.err.empty"));
            return;
        };
        let (eye, target) = framing::frame_bounds((cam.target - cam.eye).normalize_or_zero(), b.min, b.max, cam.fov_y, aspect, 1.1);
        self.render_apply(Command::SetCamera { eye: Some(eye.to_array()), target: Some(target.to_array()), up: Some([0.0, 1.0, 0.0]), fov_degrees: None, projection: None });
    }

    // ============================================================ Passes

    fn passes_tab(&mut self, ui: &mut egui::Ui) {
        let project = self.core.project();
        let render = project.render.clone();
        let out = render.output;
        let disabled = render.graph_disabled.clone();

        section(ui, icons::STACK, "render.passes.graph", true, |ui| {
            ui.label(dim(tr("render.passes.graph_hint")));
            let mut outline_on = !disabled.iter().any(|p| p == "outline");
            if ui.checkbox(&mut outline_on, tr("render.pass.outline")).on_hover_text(tr("render.pass.outline_tip")).changed() {
                self.set_pass_disabled("outline", !outline_on);
            }
            let mut cel = true;
            ui.add_enabled(false, egui::Checkbox::new(&mut cel, tr("render.pass.cel"))).on_hover_text(tr("render.pass.cel_tip"));
            let mut dof = out.dof;
            let mut dof_changed = false;
            if ui.checkbox(&mut dof.enabled, tr("render.pass.dof")).on_hover_text(tr("render.pass.dof_tip")).changed() {
                dof_changed = true;
            }
            if dof.enabled {
                ui.indent("dof_params", |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(tr("render.dof.focus"));
                        dof_changed |= ui.add(egui::DragValue::new(&mut dof.focus_distance).speed(0.05).range(0.05..=1000.0).suffix(" m").max_decimals(2)).changed();
                        if ui.small_button(icons::TARGET).on_hover_text(tr("render.dof.focus_sel")).clicked() {
                            if let Some(d) = self.render_focus_distance() {
                                dof.focus_distance = d;
                                dof_changed = true;
                            }
                        }
                    });
                    ui.horizontal_wrapped(|ui| {
                        ui.label(tr("render.dof.fnumber"));
                        dof_changed |= ui.add(egui::DragValue::new(&mut dof.f_number).speed(0.05).range(0.5..=64.0).prefix("f/").max_decimals(1)).changed();
                        ui.label(tr("render.dof.focal"));
                        dof_changed |= ui.add(egui::DragValue::new(&mut dof.focal_mm).speed(0.5).range(8.0..=800.0).suffix(" mm").max_decimals(0)).changed();
                    });
                    ui.horizontal_wrapped(|ui| {
                        ui.label(tr("render.dof.bokeh"));
                        if ui.selectable_label(dof.bokeh_shape == 0, tr("render.dof.round")).clicked() && dof.bokeh_shape != 0 {
                            dof.bokeh_shape = 0;
                            dof_changed = true;
                        }
                        if ui.selectable_label(dof.bokeh_shape == 1, tr("render.dof.hex")).clicked() && dof.bokeh_shape != 1 {
                            dof.bokeh_shape = 1;
                            dof_changed = true;
                        }
                        ui.label(tr("render.dof.max_radius"));
                        dof_changed |= ui.add(egui::DragValue::new(&mut dof.max_radius_px).speed(0.2).range(0.0..=64.0).suffix(" px").max_decimals(0)).changed();
                    });
                });
            }
            if dof_changed {
                self.render_edit_patch(RenderOutputPatch { dof: Some(dof), ..Default::default() });
            }
            let mut prepass = render.depth_prepass;
            if ui.checkbox(&mut prepass, tr("render.pass.prepass")).on_hover_text(tr("render.pass.prepass_tip")).changed() {
                self.render_apply(Command::SetRenderSettings { msaa_samples: None, tonemap: None, graph_order: None, graph_disabled: None, depth_prepass: Some(prepass) });
            }
            ui.add_space(4.0);
            ui.label(dim(format!("MSAA: 4\u{d7} ({})", tr("render.contract"))));
            ui.label(dim(format!("{}: {}", tr("render.color.tonemap"), match render.tonemap {
                TonemapOperator::None => tr("render.color.tonemap_none"),
                TonemapOperator::Reinhard => "Reinhard",
                TonemapOperator::Neutral => "Neutral",
            })));
            ui.label(dim(tr("render.color.tonemap_tip")));
        });

        section(ui, icons::SQUARES_FOUR, "render.passes.aov", true, |ui| {
            ui.label(dim(tr("render.passes.aov_hint")));
            let mut a: AovSelection = out.aov;
            let mut changed = false;
            changed |= ui.checkbox(&mut a.outline, tr("render.aov.outline")).on_hover_text(tr("render.aov.outline_tip")).changed();
            changed |= ui.checkbox(&mut a.depth, tr("render.aov.depth")).on_hover_text(tr("render.aov.depth_tip")).changed();
            changed |= ui.checkbox(&mut a.mask, tr("render.aov.mask")).on_hover_text(tr("render.aov.mask_tip")).changed();
            changed |= ui.checkbox(&mut a.object_id, tr("render.aov.id")).on_hover_text(tr("render.aov.id_tip")).changed();
            changed |= ui.checkbox(&mut a.normal, tr("render.aov.normal")).on_hover_text(tr("render.aov.normal_tip")).changed();
            if changed {
                self.render_apply_patch(RenderOutputPatch { aov: Some(a), ..Default::default() });
            }
            if a.outline && disabled.iter().any(|p| p == "outline") {
                ui.label(RichText::new(tr("render.aov.outline_off")).size(11.5).color(WARN));
            }
            ui.label(dim(tr("render.aov.note")));
        });

        section(ui, icons::SPARKLE, "render.passes.post", false, |ui| {
            for k in ["render.post.bloom", "render.post.grain", "render.post.lut"] {
                let mut off = false;
                ui.add_enabled(false, egui::Checkbox::new(&mut off, tr(k)));
            }
            ui.label(dim(tr("render.post.note")));
        });
    }

    fn set_pass_disabled(&mut self, name: &str, disable: bool) {
        let mut list = self.core.project().render.graph_disabled.clone();
        list.retain(|p| p != name);
        if disable {
            list.push(name.to_string());
        }
        self.render_apply(Command::SetRenderSettings { msaa_samples: None, tonemap: None, graph_order: None, graph_disabled: Some(list), depth_prepass: None });
    }

    /// Distância da câmera ao centro da seleção (ou da cena) — para focar o DoF.
    pub(crate) fn render_focus_distance(&self) -> Option<f32> {
        let cam = &self.core.project().scene.camera.camera;
        let su = &self.scene_ui;
        let b = if su.selection.is_empty() { su.runtime.bounds } else { su.selection.ids().iter().filter_map(|i| su.runtime.bounds_of(i)).reduce(|a, b| a.union(&b)) }?;
        let center = (b.min + b.max) * 0.5;
        Some(((center - cam.eye).dot((cam.target - cam.eye).normalize_or_zero())).max(0.05))
    }

    // ============================================================ Saída

    fn output_tab(&mut self, ui: &mut egui::Ui) {
        let project = self.core.project();
        let out = project.render.output;
        let bg = project.render.background_color;
        let space = project.render.color.display_space;

        section(ui, icons::FRAME_CORNERS, "render.res", true, |ui| {
            let current = settings::preset_for(out.width, out.height);
            let label = match current {
                Some(p) => format!("{}  {}\u{d7}{}", tr(p.key), p.w, p.h),
                None => format!("{}  {}\u{d7}{}", tr("render.preset.custom"), out.width, out.height),
            };
            let mut picked = None;
            crate::app::props_tabs::combo(ui, "render_presets").selected_text(label).width(ui.available_width() - 8.0).height(420.0).show_ui(ui, |ui| {
                for g in PresetGroup::ALL {
                    ui.label(RichText::new(tr(g.key())).strong().color(theme::ACCENT));
                    for p in PRESETS.iter().filter(|p| p.group == g) {
                        let on = current.is_some_and(|c| c.key == p.key);
                        if ui.selectable_label(on, format!("{}   {}\u{d7}{}  ·  {} dpi", tr(p.key), p.w, p.h, p.dpi)).clicked() {
                            picked = Some(*p);
                        }
                    }
                    ui.separator();
                }
            });
            if let Some(p) = picked {
                self.render_ui.aspect_ref = p.w as f32 / p.h as f32;
                self.render_apply_patch(RenderOutputPatch { width: Some(p.w), height: Some(p.h), dpi: Some(p.dpi), ..Default::default() });
            }
            ui.horizontal_wrapped(|ui| {
                let (mut w, mut h) = (out.width, out.height);
                let lock = self.render_ui.prefs.lock_aspect;
                let mut edit: Option<(u32, u32)> = None;
                if ui.add(egui::DragValue::new(&mut w).speed(4.0).range(16..=16384).suffix(" px")).changed() {
                    edit = Some(if lock { settings::lock_aspect(Axis::Width, w, self.render_ui.aspect_ref, 16, 16384) } else { (w, out.height) });
                }
                let lock_icon = if lock { icons::LOCK_SIMPLE } else { icons::LOCK_SIMPLE_OPEN };
                if ui.selectable_label(lock, lock_icon).on_hover_text(tr("render.res.lock")).clicked() {
                    self.render_ui.prefs.lock_aspect = !lock;
                    self.render_ui.aspect_ref = out.aspect();
                }
                if ui.add(egui::DragValue::new(&mut h).speed(4.0).range(16..=16384).suffix(" px")).changed() {
                    edit = Some(if lock { settings::lock_aspect(Axis::Height, h, self.render_ui.aspect_ref, 16, 16384) } else { (out.width, h) });
                }
                let swap = ui.button(icons::ARROWS_LEFT_RIGHT).on_hover_text(tr("render.res.swap"));
                self.render_ui.swap_btn = swap.rect.center();
                if swap.clicked() {
                    self.render_ui.aspect_ref = out.height as f32 / out.width as f32;
                    self.render_apply_patch(RenderOutputPatch { width: Some(out.height), height: Some(out.width), ..Default::default() });
                }
                if let Some((w, h)) = edit {
                    self.render_edit_patch(RenderOutputPatch { width: Some(w), height: Some(h), ..Default::default() });
                }
            });
            ui.horizontal_wrapped(|ui| {
                ui.label(tr("render.res.scale"));
                let mut s = out.scale_percent;
                if ui.add(egui::DragValue::new(&mut s).speed(1.0).range(10..=400).suffix(" %")).changed() {
                    self.render_edit_patch(RenderOutputPatch { scale_percent: Some(s), ..Default::default() });
                }
                ui.label(dim(format!("{}", settings::aspect_label(out.width, out.height))));
            });
            match settings::plan(&out, 8192, DEFAULT_MAX_PIXELS) {
                Ok(p) => {
                    ui.label(dim(format!("{}: {}\u{d7}{}  \u{b7}  {}: {}\u{d7}{} (SS {}\u{d7})", tr("render.res.file"), p.target.0, p.target.1, tr("render.res.target"), p.render_size.0, p.render_size.1, p.ss)));
                    if p.clamped() {
                        ui.label(RichText::new(format!("{} {}\u{d7} \u{2192} {}\u{d7}", tr("render.res.ss_clamped"), p.ss_requested, p.ss)).size(11.5).color(WARN));
                    }
                }
                Err(e) => {
                    ui.label(RichText::new(e).size(11.5).color(ERR));
                }
            }
        });

        section(ui, icons::SPARKLE, "render.aa", true, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(tr("render.aa.ss"));
                for (s, label) in [(1u32, tr("render.aa.off")), (2, "2\u{d7}"), (3, "3\u{d7}"), (4, "4\u{d7}")] {
                    if ui.selectable_label(out.supersample == s, label).clicked() && out.supersample != s {
                        self.render_apply_patch(RenderOutputPatch { supersample: Some(s), ..Default::default() });
                    }
                }
            });
            ui.label(dim(format!("MSAA: 4\u{d7} ({})  \u{b7}  {}", tr("render.contract"), tr("render.aa.ss_tip"))));
        });

        section(ui, icons::PALETTE, "render.bg", true, |ui| {
            let mut t = out.transparent;
            if ui.checkbox(&mut t, tr("render.bg.transparent")).on_hover_text(tr("render.bg.transparent_tip")).changed() {
                self.render_apply_patch(RenderOutputPatch { transparent: Some(t), ..Default::default() });
            }
            ui.horizontal_wrapped(|ui| {
                ui.label(tr("render.bg.color"));
                let mut c = [bg[0], bg[1], bg[2]];
                if ui.color_edit_button_rgb(&mut c).changed() {
                    self.render_edit(Command::SetBackgroundColor { color: [c[0], c[1], c[2], 1.0] });
                }
                if out.transparent {
                    ui.label(dim(tr("render.bg.color_unused")));
                }
            });
        });

        section(ui, icons::SLIDERS, "render.color", true, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(tr("render.color.space"));
                for (s, label) in [(ColorSpace::Srgb, "sRGB"), (ColorSpace::DisplayP3, "Display P3")] {
                    if ui.selectable_label(space == s, label).clicked() && space != s {
                        self.render_apply_patch(RenderOutputPatch { display_space: Some(s), ..Default::default() });
                    }
                }
            });
            ui.label(dim(tr(if space == ColorSpace::DisplayP3 { "render.color.p3_tip" } else { "render.color.srgb_tip" })));
        });

        section(ui, icons::FILE_IMAGE, "render.format", true, |ui| {
            ui.horizontal_wrapped(|ui| {
                for (f, label) in [(OutputFormat::Png8, "PNG 8"), (OutputFormat::Png16, "PNG 16"), (OutputFormat::Jpeg, "JPEG")] {
                    if ui.selectable_label(out.format == f, label).clicked() && out.format != f {
                        self.render_apply_patch(RenderOutputPatch { format: Some(f), ..Default::default() });
                    }
                }
            });
            match out.format {
                OutputFormat::Png16 => {
                    ui.label(dim(tr("render.format.png16_tip")));
                }
                OutputFormat::Jpeg => {
                    let mut q = out.jpeg_quality as u32;
                    ui.horizontal_wrapped(|ui| {
                        ui.label(tr("render.format.quality"));
                        if ui.add(egui::DragValue::new(&mut q).range(1..=100).speed(0.5)).changed() {
                            self.render_edit_patch(RenderOutputPatch { jpeg_quality: Some(q as u8), ..Default::default() });
                        }
                    });
                    if out.transparent {
                        ui.label(RichText::new(tr("render.format.jpeg_alpha")).size(11.5).color(WARN));
                    }
                }
                OutputFormat::Png8 => {}
            }
            ui.horizontal_wrapped(|ui| {
                ui.label(tr("render.format.dpi"));
                let mut d = out.dpi;
                if ui.add(egui::DragValue::new(&mut d).range(36..=1200).speed(1.0)).on_hover_text(tr("render.format.dpi_tip")).changed() {
                    self.render_edit_patch(RenderOutputPatch { dpi: Some(d), ..Default::default() });
                }
            });
        });

        section(ui, icons::FOLDER_OPEN, "render.files", true, |ui| self.files_section(ui));
        section(ui, icons::FILM_STRIP, "render.sequence", false, |ui| self.sequence_section(ui));
    }

    fn files_section(&mut self, ui: &mut egui::Ui) {
        ui.label(tr("render.files.dir"));
        ui.horizontal_wrapped(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.render_ui.prefs.dir).desired_width(ui.available_width() - 74.0).hint_text(settings::suggested_output_dir().display().to_string()));
            if ui.button(tr("render.files.suggest")).on_hover_text(settings::suggested_output_dir().display().to_string()).clicked() {
                self.render_ui.prefs.dir = settings::suggested_output_dir().display().to_string();
            }
        });
        let dir = self.render_ui.out_dir();
        let (text, color) = match dir_status(&dir) {
            DirStatus::Ready => (tr("render.files.dir_ok"), OK),
            DirStatus::WillCreate => (tr("render.files.dir_create"), WARN),
            DirStatus::NotADirectory => (tr("render.files.dir_file"), ERR),
            DirStatus::ReadOnly => (tr("render.files.dir_ro"), ERR),
        };
        ui.label(RichText::new(text).size(11.5).color(color));
        ui.add_space(2.0);
        ui.label(tr("render.files.template"));
        ui.add(egui::TextEdit::singleline(&mut self.render_ui.prefs.template).desired_width(f32::INFINITY).hint_text(settings::DEFAULT_TEMPLATE)).on_hover_text(tr("render.files.template_tip"));
        ui.horizontal_wrapped(|ui| {
            ui.label(tr("render.files.counter"));
            ui.add(egui::DragValue::new(&mut self.render_ui.prefs.counter).range(1..=999_999).speed(0.2));
            ui.checkbox(&mut self.render_ui.prefs.overwrite, tr("render.files.overwrite")).on_hover_text(tr("render.files.overwrite_tip"));
        });
        // nome resultante (só leitura do disco: nada é criado)
        let out = self.core.project().render.output;
        let (tw, th) = out.target_size();
        let tokens = settings::NameTokens {
            project: self.core.project().name.clone(),
            camera: self.render_camera_name(),
            w: tw,
            h: th,
            fmt: out.format.extension().to_string(),
            ..Default::default()
        };
        let aovs = self.render_aovs_enabled();
        let a = settings::allocate(&dir, &self.render_ui.prefs.template, &tokens, out.format.extension(), &aovs, self.render_ui.prefs.overwrite, self.render_ui.prefs.counter);
        ui.label(dim(format!("\u{2192} {}", a.main.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default())));
        for (k, p) in &a.aovs {
            ui.label(dim(format!("    + {}  ({k})", p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default())));
        }
    }

    fn sequence_section(&mut self, ui: &mut egui::Ui) {
        ui.label(dim(tr("render.sequence.hint")));
        ui.add_enabled_ui(false, |ui| {
            ui.horizontal_wrapped(|ui| {
                let (mut a, mut b, mut s) = (1u32, 24u32, 1u32);
                ui.label(tr("render.sequence.range"));
                ui.add(egui::DragValue::new(&mut a));
                ui.label("\u{2192}");
                ui.add(egui::DragValue::new(&mut b));
                ui.label(tr("render.sequence.step"));
                ui.add(egui::DragValue::new(&mut s));
            });
        });
        ui.label(dim(tr("render.sequence.tokens")));
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(tr("render.video")).strong());
            if ui.button(tr("render.video.detect")).clicked() {
                self.render_ui.ffmpeg = Some(detect_ffmpeg());
            }
        });
        match &self.render_ui.ffmpeg {
            None => {}
            Some(Ok(v)) => {
                ui.label(RichText::new(format!("{} {v}", icons::CHECK_CIRCLE)).size(11.5).color(OK));
            }
            Some(Err(_)) => {
                ui.label(RichText::new(tr("render.video.missing")).size(11.5).color(WARN));
            }
        }
        let stem = settings::sanitize_file_name(&self.core.project().name);
        let mut cmd = ffmpeg_command(&self.render_ui.out_dir(), &stem, 24);
        ui.add(egui::TextEdit::multiline(&mut cmd).desired_rows(2).desired_width(f32::INFINITY).font(egui::TextStyle::Monospace));
        ui.label(dim(tr("render.video.note")));
    }

    // ============================================================ Fila e histórico

    fn queue_panel(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(format!("{}  {}", icons::LIST_CHECKS, tr("render.queue"))).strong());
            ui.label(dim(format!("{}", self.render_ui.jobs.len())));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button(icons::CARET_DOWN).on_hover_text(tr("render.queue.hide")).clicked() {
                    self.render_ui.prefs.queue_open = false;
                }
                if ui.small_button(icons::BROOM).on_hover_text(tr("render.queue.clear")).clicked() {
                    self.render_ui.jobs.retain(|j| matches!(j.status, JobStatus::Queued | JobStatus::Running));
                }
            });
        });
        ui.separator();
        let mut cancel = None;
        let mut resume = None;
        let mut first_running = true;
        egui::ScrollArea::vertical().id_salt("render_jobs").max_height(ui.available_height() * 0.45).show(ui, |ui| {
            if self.render_ui.jobs.is_empty() {
                ui.label(dim(tr("render.queue.empty")));
            }
            for j in self.render_ui.jobs.iter().rev() {
                ui.horizontal_wrapped(|ui| {
                    let (icon, color) = match &j.status {
                        JobStatus::Queued => (icons::CLOCK, theme::TEXT_DIM),
                        JobStatus::Running => (icons::SPINNER_GAP, theme::ACCENT),
                        JobStatus::Done => (icons::CHECK_CIRCLE, OK),
                        JobStatus::Failed(_) => (icons::WARNING_CIRCLE, ERR),
                        JobStatus::Cancelled => (icons::PROHIBIT, WARN),
                    };
                    ui.label(RichText::new(icon).color(color));
                    ui.label(RichText::new(&j.label).strong().size(12.5));
                    match &j.status {
                        JobStatus::Running | JobStatus::Queued => {
                            let stage = j.stage.map(|s| tr(stage_key(s)).to_string()).unwrap_or_else(|| tr("render.queue.waiting").to_string());
                            ui.add(egui::ProgressBar::new(j.frac.clamp(0.0, 1.0)).desired_width(150.0).show_percentage());
                            ui.label(dim(if j.total > 1 { format!("{stage}  {}/{}", j.frame_idx + 1, j.total) } else { stage }));
                            let b = ui.small_button(icons::X).on_hover_text(tr("render.queue.cancel"));
                            if first_running {
                                self.render_ui.cancel_btn = b.rect.center();
                                first_running = false;
                            }
                            if b.clicked() {
                                cancel = Some(j.id);
                            }
                        }
                        JobStatus::Done => {
                            ui.label(dim(format!("{} \u{b7} {:.1}s \u{b7} {} {}", fmt_bytes(j.files.iter().map(|f| f.bytes).sum()), j.finished.map(|d| d.as_secs_f32()).unwrap_or(0.0), j.files.len(), tr("render.queue.files"))));
                        }
                        JobStatus::Failed(e) => {
                            ui.label(RichText::new(e).size(11.5).color(ERR));
                            if ui.small_button(icons::ARROW_CLOCKWISE).on_hover_text(tr("render.queue.resume")).clicked() {
                                resume = Some(j.id);
                            }
                        }
                        JobStatus::Cancelled => {
                            ui.label(dim(tr("render.cancelled")));
                            if ui.small_button(icons::ARROW_CLOCKWISE).on_hover_text(tr("render.queue.resume")).clicked() {
                                resume = Some(j.id);
                            }
                        }
                    }
                });
                for n in &j.notes {
                    ui.label(RichText::new(format!("    {} {n}", icons::INFO)).size(11.0).color(WARN));
                }
            }
        });
        if let Some(id) = cancel {
            self.render_cancel(id);
        }
        if let Some(id) = resume {
            self.render_resume(ctx, id);
        }
        ui.separator();
        ui.label(RichText::new(format!("{}  {}", icons::IMAGES, tr("render.history"))).strong());
        self.history_strip(ui, ctx);
    }

    fn history_strip(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        // um cartão por imagem principal; os AOVs gravados ao lado entram como "+ outline, depth…"
        let all = &self.render_ui.history;
        let entries: Vec<(HistoryEntry, Vec<String>)> = all
            .iter()
            .filter(|e| e.pass == "beauty")
            .take(48)
            .map(|b| {
                let stem = b.file.rsplit_once('.').map(|(s, _)| s.to_string()).unwrap_or_else(|| b.file.clone());
                let aovs = all.iter().filter(|e| e.pass != "beauty" && e.file.starts_with(&format!("{stem}_"))).map(|e| e.pass.clone()).collect();
                (b.clone(), aovs)
            })
            .collect();
        if entries.is_empty() {
            ui.label(dim(tr("render.history.empty")));
            return;
        }
        let mut to_library = None;
        let mut copy = None;
        egui::ScrollArea::horizontal().id_salt("render_history").auto_shrink([false, false]).show(ui, |ui| {
            ui.horizontal_top(|ui| {
                for (e, aovs) in &entries {
                    let tex = self.render_thumb(ctx, e);
                    ui.allocate_ui(egui::vec2(150.0, ui.available_height().max(120.0)), |ui| {
                        ui.vertical(|ui| {
                            let (r, _) = ui.allocate_exact_size(egui::vec2(150.0, 84.0), egui::Sense::hover());
                            ui.painter().rect_filled(r, 3.0, theme::BG_DEEP);
                            if let Some(t) = &tex {
                                let sz = t.size_vec2();
                                let k = (r.width() / sz.x).min(r.height() / sz.y);
                                let img = egui::Rect::from_center_size(r.center(), sz * k);
                                ui.painter().image(t.id(), img, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
                            } else {
                                ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, icons::FILE_IMAGE, egui::FontId::proportional(26.0), theme::TEXT_DIM);
                            }
                            ui.label(RichText::new(&e.file).size(11.0)).on_hover_text(format!("{}\n{}\n{}", e.when, e.camera, e.fingerprint));
                            ui.label(dim(format!("{}\u{d7}{} \u{b7} {}", e.w, e.h, fmt_bytes(e.bytes))));
                            if !aovs.is_empty() {
                                ui.label(dim(format!("+ {}", aovs.join(", "))));
                            }
                            ui.horizontal_wrapped(|ui| {
                                if ui.small_button(icons::PACKAGE).on_hover_text(tr("render.history.to_library")).clicked() {
                                    to_library = Some(e.clone());
                                }
                                if ui.small_button(icons::COPY).on_hover_text(tr("render.history.copy_path")).clicked() {
                                    copy = Some(e.path(&self.render_ui.out_dir()).display().to_string());
                                }
                            });
                        });
                    });
                    ui.add_space(6.0);
                }
            });
        });
        if let Some(e) = to_library {
            self.render_to_library(&e);
        }
        if let Some(t) = copy {
            ctx.copy_text(t);
            self.render_say(tr("render.history.copied"));
        }
    }
}

fn fmt3(v: Vec3) -> String {
    format!("{:.2}, {:.2}, {:.2}", v.x, v.y, v.z)
}
